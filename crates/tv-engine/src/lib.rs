pub mod batch_stream;
pub mod bitmap_index;
pub mod bloom_index;
pub mod catalog;
pub mod compiler;
pub mod error;
pub mod executor;
pub mod export;
pub mod extensions;
pub mod external_sort;
pub mod index_catalog;
pub mod job_registry;
pub mod mark_index;
pub mod materializer;
pub mod profiles;
pub mod quantile_sketch;
pub mod query;
pub mod reader;
pub mod roaring_index;
pub mod sort_index;
pub mod sparse_sort_index;
pub mod spill;
pub mod spill_pipeline;
pub mod stats;
pub mod streaming_agg;
pub mod temp;
pub mod top_k;

#[cfg(test)]
pub mod test_helpers;

use arrow::array::Array;
use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use catalog::{detect_format, detect_kind, infer_name, Catalog};
use error::EngineError;
use external_sort::ExternalSorter;
use index_catalog::IndexCatalog;
use job_registry::JobRegistry;
use materializer::{MaterializedView, ViewMaterializer};
use query::{maybe_dict_encode_batch, project_tile_columns, serialize_to_arrow_ipc};
use spill_pipeline::SpillPipeline;
use std::collections::HashMap;
use std::sync::Arc;
use temp::TempRoot;
use tracing::{debug, info};
use tv_core::{
    needed_column_indices, optimize_with_quantiles, view_hash, BatchTileRequest, ColumnInfo,
    ColumnStats, CorrelationMatrix, Credentials, Literal, Predicate, QuickColumnStats,
    SourceFormat, SourceKind, SourceMeta, TileRequest, TileResponse, ViewExpr, ViewOp,
};

type FilterRgIndex = Arc<std::sync::RwLock<HashMap<(String, String), Arc<Vec<u64>>>>>;
type BloomCache = Arc<std::sync::RwLock<HashMap<String, Arc<bloom_index::BloomIndex>>>>;
type QuantileCache =
    Arc<std::sync::RwLock<HashMap<String, Arc<HashMap<String, tv_core::types::QuantileSketch>>>>>;
type SortAccessCounter = Arc<std::sync::Mutex<HashMap<String, u64>>>;
type MetadataCache =
    Arc<std::sync::RwLock<HashMap<String, Arc<parquet::file::metadata::ParquetMetaData>>>>;
type RoaringCache =
    Arc<std::sync::RwLock<HashMap<(String, usize), Arc<roaring_index::RoaringIndex>>>>;
type MarkCache = mark_index::MarkCache;

#[derive(Clone)]
pub struct Engine {
    catalog: Arc<Catalog>,
    materializer: Arc<ViewMaterializer>,
    stats_cache: Arc<std::sync::RwLock<HashMap<(String, usize), ColumnStats>>>,
    schema_cache: Arc<std::sync::RwLock<HashMap<String, SchemaRef>>>,
    metadata_cache: MetadataCache,
    filter_rg_index: FilterRgIndex,
    bloom_cache: BloomCache,
    quantile_cache: QuantileCache,
    sort_access_counter: SortAccessCounter,
    temp_root: Arc<TempRoot>,
    index_catalog: Arc<IndexCatalog>,
    roaring_cache: RoaringCache,
    mark_cache: MarkCache,
    job_registry: Arc<JobRegistry>,
}

impl Engine {
    pub fn new() -> Result<Self, EngineError> {
        Ok(Self {
            catalog: Arc::new(Catalog::new()),
            materializer: Arc::new(ViewMaterializer::new()),
            stats_cache: Arc::new(std::sync::RwLock::new(HashMap::new())),
            schema_cache: Arc::new(std::sync::RwLock::new(HashMap::new())),
            metadata_cache: Arc::new(std::sync::RwLock::new(HashMap::new())),
            filter_rg_index: Arc::new(std::sync::RwLock::new(HashMap::new())),
            bloom_cache: Arc::new(std::sync::RwLock::new(HashMap::new())),
            quantile_cache: Arc::new(std::sync::RwLock::new(HashMap::new())),
            sort_access_counter: Arc::new(std::sync::Mutex::new(HashMap::new())),
            temp_root: TempRoot::new()?,
            index_catalog: Arc::new(IndexCatalog::new()),
            roaring_cache: Arc::new(std::sync::RwLock::new(HashMap::new())),
            mark_cache: mark_index::new_mark_cache(),
            job_registry: Arc::new(JobRegistry::new()),
        })
    }

    pub fn job_registry(&self) -> Arc<JobRegistry> {
        Arc::clone(&self.job_registry)
    }

    pub async fn register_source(
        &self,
        uri: &str,
        name: Option<String>,
        _profile: Option<String>,
        _credentials: Option<Credentials>,
    ) -> Result<SourceMeta, EngineError> {
        let name = name.unwrap_or_else(|| infer_name(uri));
        let kind = detect_kind(uri);
        let format = detect_format(uri, &kind);
        let id = uuid::Uuid::new_v4().to_string();

        let files = expand_source_files(uri, &kind).await?;
        let primary = files.first().map(|s| s.as_str()).unwrap_or(uri);

        let (schema, n_rows) = inspect_source(primary, &format, &files, &kind).await?;

        if files.len() > 1 && matches!(format, SourceFormat::Parquet) && !is_cloud_kind(&kind) {
            for path in &files[1..] {
                let (file_schema, _) = reader::parquet_schema_and_rows(path)?;
                if file_schema.fields() != schema.fields() {
                    return Err(EngineError::Query(format!(
                        "schema mismatch: {path} schema differs from primary file"
                    )));
                }
            }
        }

        let columns: Vec<ColumnInfo> = schema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, f)| ColumnInfo {
                index: i,
                name: f.name().clone(),
                data_type: format_data_type(f.data_type()),
                nullable: f.is_nullable(),
            })
            .collect();

        let n_cols = columns.len();
        let quick_stats = build_quick_stats(primary, n_rows, n_cols, &format, &kind);

        let (tile_rows, file_size_bytes, file_mtime_secs, recommendations) = if matches!(
            format,
            SourceFormat::Parquet
        ) && !is_cloud_kind(
            &kind,
        ) && files.len()
            == 1
        {
            let path = files.first().map(|s| s.as_str()).unwrap_or(uri);
            let fs_meta = std::fs::metadata(path);
            let (fsize, fmtime) = if let Ok(ref m) = fs_meta {
                let size = m.len();
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                (size, mtime)
            } else {
                (0u64, 0u64)
            };

            let (tr, recs) = if let Ok((_, _, pq_meta)) =
                reader::parquet_schema_rows_and_metadata(path)
            {
                let n_rgs = pq_meta.num_row_groups();
                let first_rg_rows = if n_rgs > 0 {
                    pq_meta.row_group(0).num_rows() as u64
                } else {
                    0
                };
                let tile_r = if first_rg_rows > 0 {
                    tv_core::optimal_tile_rows(first_rg_rows)
                } else {
                    256
                };

                let mut advice: Vec<tv_core::SourceRecommendation> = Vec::new();
                if n_rgs > 0 && fsize > 0 {
                    let avg_rg_bytes = fsize / n_rgs as u64;
                    if avg_rg_bytes < 64 * 1024 * 1024 {
                        advice.push(tv_core::SourceRecommendation {
                                kind: "small_row_groups".to_string(),
                                message: "Row groups are small (<64MB); rewrite with 512MB row groups for faster tile access".to_string(),
                            });
                    }
                }
                let mut missing_stats = false;
                'rg_loop: for rg_i in 0..n_rgs {
                    let rg = pq_meta.row_group(rg_i);
                    for col_i in 0..rg.num_columns() {
                        if rg.column(col_i).statistics().is_none() {
                            missing_stats = true;
                            break 'rg_loop;
                        }
                    }
                }
                if missing_stats {
                    advice.push(tv_core::SourceRecommendation {
                            kind: "missing_statistics".to_string(),
                            message: "Some columns lack min/max statistics; rewrite with statistics enabled for better row group pruning".to_string(),
                        });
                }
                let bloom_loaded = self.index_catalog.lookup_bloom_index(path).is_some();
                if n_cols > 0 && !bloom_loaded {
                    advice.push(tv_core::SourceRecommendation {
                            kind: "no_bloom_filters".to_string(),
                            message: "No bloom filter index found; build one for faster equality filter on string columns".to_string(),
                        });
                }
                (tile_r, advice)
            } else {
                (256u32, Vec::new())
            };
            (tr, fsize, fmtime, recs)
        } else {
            (256u32, 0u64, 0u64, Vec::new())
        };

        let meta_files_for_presort = files.clone();
        let format_for_presort = format.clone();
        let kind_for_presort = kind.clone();
        let schema_for_presort = schema.clone();
        let pre_sorted_by = detect_pre_sorted_by(
            &meta_files_for_presort,
            &format_for_presort,
            &kind_for_presort,
            &schema_for_presort,
        );
        let meta = SourceMeta {
            id,
            name,
            uri: uri.to_string(),
            files,
            format,
            kind,
            n_rows,
            n_cols,
            columns,
            quick_stats,
            tile_rows,
            file_size_bytes,
            file_mtime_secs,
            recommendations,
            pre_sorted_by,
        };
        info!(
            id = %meta.id,
            name = %meta.name,
            n_rows = meta.n_rows,
            n_cols = meta.n_cols,
            format = ?meta.format,
            "source registered"
        );
        self.schema_cache
            .write()
            .unwrap()
            .insert(meta.id.clone(), schema);
        self.catalog.insert(meta.clone());
        if matches!(&meta.format, SourceFormat::Parquet) && !is_cloud_kind(&meta.kind) {
            let source_path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
            if meta.files.len() == 1 {
                if let Ok((_, _, pq_meta)) = reader::parquet_schema_rows_and_metadata(source_path) {
                    self.metadata_cache
                        .write()
                        .unwrap()
                        .insert(source_path.to_string(), pq_meta);
                }
            }
            self.index_catalog.scan_for_source(source_path);
            if let Some(bloom_path) = self.index_catalog.lookup_bloom_index(source_path) {
                if let Ok(bloom) = bloom_index::load(&bloom_path) {
                    self.bloom_cache
                        .write()
                        .unwrap()
                        .insert(source_path.to_string(), Arc::new(bloom));
                }
            }
            if let Some(q_path) = self.index_catalog.lookup_quantile_index(source_path) {
                if let Ok(q_index) = quantile_sketch::load_quantile_index(&q_path) {
                    let schema_ref = self.schema_cache.read().unwrap().get(&meta.id).cloned();
                    if let Some(schema_ref) = schema_ref {
                        let sketches =
                            quantile_sketch::tdigest_to_global_sketches(&q_index, &schema_ref);
                        self.quantile_cache
                            .write()
                            .unwrap()
                            .insert(source_path.to_string(), Arc::new(sketches));
                    }
                }
            }
            let schema_for_roaring = self.schema_cache.read().unwrap().get(&meta.id).cloned();
            if let Some(schema_ref) = schema_for_roaring {
                for col in schema_ref.fields().iter().enumerate().filter_map(|(i, f)| {
                    if matches!(
                        f.data_type(),
                        arrow::datatypes::DataType::Utf8 | arrow::datatypes::DataType::LargeUtf8
                    ) {
                        Some(i)
                    } else {
                        None
                    }
                }) {
                    if let Some(r_path) = self.index_catalog.lookup_roaring_index(source_path, col)
                    {
                        if let Ok(ri) = roaring_index::load(&r_path.to_string_lossy()) {
                            self.roaring_cache
                                .write()
                                .unwrap()
                                .insert((source_path.to_string(), col), Arc::new(ri));
                        }
                    }
                }
            }
            let schema_for_mark = self.schema_cache.read().unwrap().get(&meta.id).cloned();
            if let Some(schema_ref) = schema_for_mark {
                for col in schema_ref.fields().iter().enumerate().filter_map(|(i, f)| {
                    if matches!(
                        f.data_type(),
                        arrow::datatypes::DataType::Int8
                            | arrow::datatypes::DataType::Int16
                            | arrow::datatypes::DataType::Int32
                            | arrow::datatypes::DataType::Int64
                            | arrow::datatypes::DataType::Float32
                            | arrow::datatypes::DataType::Float64
                            | arrow::datatypes::DataType::Date32
                            | arrow::datatypes::DataType::Date64
                    ) {
                        Some(i)
                    } else {
                        None
                    }
                }) {
                    if let Some(m_path) = self.index_catalog.lookup_mark_index(source_path, col) {
                        if let Ok(mi) = mark_index::MarkIndex::load(&m_path.to_string_lossy()) {
                            self.mark_cache
                                .write()
                                .unwrap()
                                .insert((source_path.to_string(), col), Arc::new(mi));
                        }
                    }
                }
            }
        }
        Ok(meta)
    }

    pub fn list_sources(&self) -> Vec<SourceMeta> {
        self.catalog.list()
    }

    pub fn get_source(&self, id: &str) -> Option<SourceMeta> {
        self.catalog.get(id)
    }

    pub async fn remove_source(&self, id: &str) -> Result<(), EngineError> {
        let source_path = self
            .catalog
            .get(id)
            .map(|m| m.files.first().cloned().unwrap_or(m.uri.clone()));
        if !self.catalog.remove(id) {
            return Err(EngineError::SourceNotFound(id.to_string()));
        }
        info!(id = %id, "source removed");
        self.materializer.invalidate_source(id).await;
        {
            let mut cache = self.stats_cache.write().unwrap();
            cache.retain(|(sid, _), _| sid != id);
        }
        self.schema_cache.write().unwrap().remove(id);
        {
            let mut idx = self.filter_rg_index.write().unwrap();
            idx.retain(|(sid, _), _| sid != id);
        }
        if let Some(ref path) = source_path {
            self.bloom_cache.write().unwrap().remove(path);
            self.quantile_cache.write().unwrap().remove(path);
            self.metadata_cache.write().unwrap().remove(path);
            let mut rc = self.roaring_cache.write().unwrap();
            rc.retain(|(p, _), _| p != path);
            let mut mc = self.mark_cache.write().unwrap();
            mc.retain(|(p, _), _| p != path);
        }
        {
            let mut counter = self.sort_access_counter.lock().unwrap();
            counter.retain(|k, _| !k.starts_with(id));
        }
        self.temp_root.cleanup_source(id);
        if let Some(path) = source_path {
            self.index_catalog.remove_source(&path);
        }
        self.job_registry.remove_for_source(id).await;
        Ok(())
    }

    pub fn check_source_stale(&self, id: &str) -> Option<bool> {
        let meta = self.catalog.get(id)?;
        if meta.kind != SourceKind::LocalFile || meta.files.len() != 1 {
            return None;
        }
        if meta.file_size_bytes == 0 && meta.file_mtime_secs == 0 {
            return None;
        }
        let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
        let fs_meta = std::fs::metadata(path).ok()?;
        let current_size = fs_meta.len();
        let current_mtime = fs_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(current_size != meta.file_size_bytes || current_mtime != meta.file_mtime_secs)
    }

    pub async fn query_tile(&self, req: &TileRequest) -> Result<TileResponse, EngineError> {
        let meta = self
            .catalog
            .get(&req.source_id)
            .ok_or_else(|| EngineError::SourceNotFound(req.source_id.clone()))?;

        if let Some(sort) = &req.sort {
            let cache_key = format!("{}:sort:{:?}", req.source_id, sort);
            let meta_c = meta.clone();
            let sort_c = sort.clone();
            let mat_view = self
                .materializer
                .get_or_materialize(cache_key, move || async move {
                    let all = read_full_dispatch(&meta_c, None).await?;
                    let sorted = executor::apply_sort_spec(all, &sort_c)?;
                    let total_rows: u64 = sorted.iter().map(|b| b.num_rows() as u64).sum();
                    Ok(MaterializedView::Batches {
                        batches: sorted,
                        total_rows,
                    })
                })
                .await?;

            if let MaterializedView::Batches { batches, .. } = mat_view.as_ref() {
                let total: usize = batches.iter().map(|b| b.num_rows()).sum();
                let start = req.row as usize;
                let len = (req.rows as usize).min(total.saturating_sub(start));
                let sliced = slice_batches(batches, start, len);
                let projected: Vec<RecordBatch> = sliced
                    .iter()
                    .map(|b| project_tile_columns(b, req.col, req.cols))
                    .collect::<Result<_, _>>()?;
                let data = serialize_to_arrow_ipc(&projected)?;
                return Ok(TileResponse {
                    source_id: req.source_id.clone(),
                    row: req.row,
                    col: req.col,
                    data,
                    is_provisional: false,
                    job_id: None,
                });
            }
        }

        let col_end = (req.col + req.cols).min(meta.n_cols);
        let col_indices: Vec<usize> = (req.col..col_end).collect();
        let tile_meta_arc = if matches!(meta.format, SourceFormat::Parquet)
            && !is_cloud_kind(&meta.kind)
            && meta.files.len() <= 1
        {
            let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
            self.metadata_cache.read().unwrap().get(path).cloned()
        } else {
            None
        };
        let mut batches = read_tile_dispatch(
            &meta,
            req.row as usize,
            &col_indices,
            req.rows as usize,
            tile_meta_arc,
        )
        .await?;

        if let Some(filter) = &req.filter {
            batches = executor::apply_filter_expr(batches, filter)?;
        }

        let data = serialize_to_arrow_ipc(&batches)?;
        Ok(TileResponse {
            source_id: req.source_id.clone(),
            row: req.row,
            col: req.col,
            data,
            is_provisional: false,
            job_id: None,
        })
    }

    pub async fn query_view_tile(
        &self,
        expr: &ViewExpr,
        row: u64,
        col_offset: usize,
        rows: u64,
        cols: usize,
    ) -> Result<TileResponse, EngineError> {
        if let Some(true) = self.check_source_stale(&expr.source_id) {
            return Err(EngineError::Query("source_modified".into()));
        }

        let meta = self
            .catalog
            .get(&expr.source_id)
            .ok_or_else(|| EngineError::SourceNotFound(expr.source_id.clone()))?;

        let stats_for_opt: Vec<ColumnStats> = {
            let cache = self.stats_cache.read().unwrap();
            let id = &meta.id;
            let mut by_col: Vec<Option<ColumnStats>> = vec![None; meta.n_cols];
            for ((sid, col_idx), stats) in cache.iter() {
                if sid == id && *col_idx < meta.n_cols {
                    by_col[*col_idx] = Some(stats.clone());
                }
            }
            by_col.into_iter().flatten().collect()
        };
        let stats_hint = if stats_for_opt.is_empty() {
            None
        } else {
            Some(stats_for_opt.as_slice())
        };
        let source_path_for_q = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
        let quantile_arc = self
            .quantile_cache
            .read()
            .unwrap()
            .get(source_path_for_q)
            .cloned();
        let optimized = optimize_with_quantiles(&expr.ops, stats_hint, quantile_arc.as_deref());
        let normalized = tv_core::normalize_ops(&optimized);
        let norm_hash = view_hash(&format!("{:?}", normalized));

        if matches!(meta.format, SourceFormat::Parquet)
            && !is_cloud_kind(&meta.kind)
            && meta.files.len() == 1
        {
            if let Some(sort_keys) = normalized.iter().find_map(|op| {
                if let ViewOp::Sort { keys } = op {
                    Some(keys.clone())
                } else {
                    None
                }
            }) {
                let counter_key = format!("{}:{:?}", meta.id, sort_keys);
                let should_build = {
                    let mut counter = self.sort_access_counter.lock().unwrap();
                    let cnt = counter.entry(counter_key).or_insert(0);
                    *cnt += 1;
                    *cnt == 1
                };
                if should_build {
                    let cache_key_bg = format!("{}:{}", expr.source_id, norm_hash);
                    let already_cached = self.materializer.get(&cache_key_bg).await.is_some();
                    if !already_cached {
                        let meta_bg = meta.clone();
                        let ops_bg = normalized.clone();
                        let schema_hint_bg =
                            self.schema_cache.read().unwrap().get(&meta.id).cloned();
                        let temp_root_bg = self.temp_root.clone();
                        let materializer_bg = Arc::clone(&self.materializer);
                        let index_catalog_bg = Arc::clone(&self.index_catalog);
                        let source_path_bg = meta
                            .files
                            .first()
                            .cloned()
                            .unwrap_or_else(|| meta.uri.clone());
                        let sort_keys_bg: Option<Vec<tv_core::SortKey>> =
                            ops_bg.iter().find_map(|op| {
                                if let ViewOp::Sort { keys } = op {
                                    Some(keys.clone())
                                } else {
                                    None
                                }
                            });
                        tokio::spawn(async move {
                            if let Ok(Ok(full_view)) = tokio::task::spawn_blocking(move || {
                                SpillPipeline::new(temp_root_bg).build_full(
                                    &meta_bg,
                                    &ops_bg,
                                    schema_hint_bg,
                                )
                            })
                            .await
                            {
                                let sparse_spill =
                                    if let MaterializedView::SparseSortIndexBacked {
                                        ref spill_path,
                                        ..
                                    } = full_view
                                    {
                                        Some(spill_path.clone())
                                    } else {
                                        None
                                    };
                                if let Some(spill) = sparse_spill {
                                    if let Some(sort_keys) = sort_keys_bg {
                                        if let Ok(tvs_path) = index_catalog_bg
                                            .register_sparse_sort_index(&source_path_bg, &sort_keys)
                                        {
                                            let _ =
                                                sparse_sort_index::build_sparse(&spill, &tvs_path);
                                        }
                                    }
                                }
                                materializer_bg.replace(cache_key_bg, full_view).await;
                            }
                        });
                    }
                }
            }
        }

        let pipeline_class = classify_pipeline(&normalized);
        debug!(
            source = %expr.source_id,
            row = row,
            col = col_offset,
            class = ?pipeline_class,
            "tile request"
        );
        match pipeline_class {
            PipelineClass::PureRead => {
                let col_end = (col_offset + cols).min(meta.n_cols);
                let col_indices: Vec<usize> = (col_offset..col_end).collect();
                let pure_metadata_arc = if matches!(meta.format, SourceFormat::Parquet)
                    && !is_cloud_kind(&meta.kind)
                    && meta.files.len() <= 1
                {
                    let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
                    self.metadata_cache.read().unwrap().get(path).cloned()
                } else {
                    None
                };
                let batches = read_tile_dispatch(
                    &meta,
                    row as usize,
                    &col_indices,
                    rows as usize,
                    pure_metadata_arc,
                )
                .await?;
                let dict_mask = if let Some(first) = batches.first() {
                    self.build_dict_mask(&expr.source_id, col_offset, &first.schema())
                } else {
                    vec![]
                };
                let encoded: Vec<RecordBatch> = if dict_mask.iter().any(|&b| b) {
                    batches
                        .iter()
                        .map(|b| maybe_dict_encode_batch(b, &dict_mask))
                        .collect::<Result<_, _>>()?
                } else {
                    batches
                };
                let data = serialize_to_arrow_ipc(&encoded)?;
                Ok(TileResponse {
                    source_id: expr.source_id.clone(),
                    row,
                    col: col_offset,
                    data,
                    is_provisional: false,
                    job_id: None,
                })
            }

            PipelineClass::StatelessOnly => {
                let filter_pred = extract_combined_filter(&normalized);
                if let Some(pred) = filter_pred {
                    if matches!(meta.format, SourceFormat::Parquet) && !is_cloud_kind(&meta.kind) {
                        let schema = match self.schema_cache.read().unwrap().get(&meta.id).cloned()
                        {
                            Some(s) => s,
                            None => {
                                let path =
                                    meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
                                reader::parquet_schema_and_rows(path).map(|(s, _)| s)?
                            }
                        };

                        if meta.files.len() <= 1 {
                            let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
                            let bloom_arc = self.bloom_cache.read().unwrap().get(path).cloned();
                            let metadata_arc =
                                self.metadata_cache.read().unwrap().get(path).cloned();
                            let roaring_arc =
                                if let Some(pred_col) = pred_col_for_roaring(&pred, &schema) {
                                    self.roaring_cache
                                        .read()
                                        .unwrap()
                                        .get(&(path.to_string(), pred_col))
                                        .cloned()
                                } else {
                                    None
                                };
                            let index_key = (meta.id.clone(), norm_hash.clone());
                            let existing_index = self
                                .filter_rg_index
                                .read()
                                .unwrap()
                                .get(&index_key)
                                .cloned();

                            let col_end = (col_offset + cols).min(schema.fields().len());
                            let display_indices: Vec<usize> = (col_offset..col_end).collect();
                            let op_indices = needed_column_indices(&normalized, &schema);
                            let col_selection: Option<Vec<usize>> =
                                if let Some(mut op_idx) = op_indices {
                                    for &di in &display_indices {
                                        if !op_idx.contains(&di) {
                                            op_idx.push(di);
                                        }
                                    }
                                    op_idx.sort_unstable();
                                    op_idx.dedup();
                                    if op_idx.len() == schema.fields().len() {
                                        None
                                    } else {
                                        Some(op_idx)
                                    }
                                } else {
                                    None
                                };

                            let mark_rgs =
                                mark_qualifying_rgs(&pred, &schema, path, &self.mark_cache);
                            let batches = if let Some(rg_index) = existing_index {
                                let path_r = path.to_string();
                                let pred_r = pred.clone();
                                let schema_r = Arc::clone(&schema);
                                let bloom_r = bloom_arc.clone();
                                let roaring_r = roaring_arc.clone();
                                let col_sel_r = col_selection.clone();
                                let mark_r = mark_rgs.clone();
                                let row_r = row as usize;
                                let rows_r = rows as usize;
                                tokio::task::spawn_blocking(move || {
                                    reader::read_parquet_filtered_tile_indexed(
                                        &path_r,
                                        &pred_r,
                                        &schema_r,
                                        bloom_r.as_deref(),
                                        roaring_r.as_deref(),
                                        row_r,
                                        rows_r,
                                        &rg_index,
                                        col_sel_r.as_deref(),
                                        mark_r.as_deref(),
                                    )
                                })
                                .await
                                .map_err(|e| EngineError::Query(e.to_string()))??
                            } else {
                                let path_r = path.to_string();
                                let pred_r = pred.clone();
                                let schema_r = Arc::clone(&schema);
                                let bloom_r = bloom_arc.clone();
                                let roaring_r = roaring_arc.clone();
                                let metadata_r = metadata_arc.clone();
                                let col_sel_r = col_selection.clone();
                                let mark_r = mark_rgs.clone();
                                let row_r = row as usize;
                                let rows_r = rows as usize;
                                let result = tokio::task::spawn_blocking(move || {
                                    reader::read_parquet_filtered_tile(
                                        &path_r,
                                        &pred_r,
                                        &schema_r,
                                        bloom_r.as_deref(),
                                        roaring_r.as_deref(),
                                        row_r,
                                        rows_r,
                                        metadata_r,
                                        col_sel_r.as_deref(),
                                        mark_r.as_deref(),
                                    )
                                })
                                .await
                                .map_err(|e| EngineError::Query(e.to_string()))??;
                                let filter_rg_index = Arc::clone(&self.filter_rg_index);
                                let path_owned = path.to_string();
                                let pred_c = pred.clone();
                                let schema_c = Arc::clone(&schema);
                                let key_c = index_key;
                                let bloom_bg = bloom_arc.clone();
                                let roaring_bg = roaring_arc.clone();
                                let metadata_bg = metadata_arc.clone();
                                tokio::task::spawn_blocking(move || {
                                    if let Ok(index) = reader::build_filter_rg_index(
                                        &path_owned,
                                        &pred_c,
                                        &schema_c,
                                        bloom_bg.as_deref(),
                                        roaring_bg.as_deref(),
                                        metadata_bg,
                                    ) {
                                        filter_rg_index
                                            .write()
                                            .unwrap()
                                            .insert(key_c, Arc::new(index));
                                    }
                                });
                                result
                            };

                            let processed =
                                executor::execute_pipeline_skip_filter(batches, &normalized)?;
                            let projected: Vec<RecordBatch> = if col_selection.is_some() {
                                let display_names: Vec<String> = (col_offset..col_end)
                                    .map(|i| schema.field(i).name().clone())
                                    .collect();
                                processed
                                    .iter()
                                    .map(|b| {
                                        project_batch_by_names(b, &display_names)
                                            .map_err(EngineError::Arrow)
                                    })
                                    .collect::<Result<_, _>>()?
                            } else {
                                processed
                                    .iter()
                                    .map(|b| project_tile_columns(b, col_offset, cols))
                                    .collect::<Result<_, _>>()?
                            };
                            let dict_mask = if let Some(first) = projected.first() {
                                self.build_dict_mask(&expr.source_id, col_offset, &first.schema())
                            } else {
                                vec![]
                            };
                            let encoded: Vec<RecordBatch> = if dict_mask.iter().any(|&b| b) {
                                projected
                                    .iter()
                                    .map(|b| maybe_dict_encode_batch(b, &dict_mask))
                                    .collect::<Result<_, _>>()?
                            } else {
                                projected
                            };
                            let data = serialize_to_arrow_ipc(&encoded)?;
                            return Ok(TileResponse {
                                source_id: expr.source_id.clone(),
                                row,
                                col: col_offset,
                                data,
                                is_provisional: false,
                                job_id: None,
                            });
                        } else {
                            let batches = reader::read_filtered_tile_multifile(
                                &meta.files,
                                &pred,
                                &schema,
                                &normalized,
                                row as usize,
                                rows as usize,
                            )?;
                            let projected: Vec<RecordBatch> = batches
                                .iter()
                                .map(|b| project_tile_columns(b, col_offset, cols))
                                .collect::<Result<_, _>>()?;
                            let dict_mask = if let Some(first) = projected.first() {
                                self.build_dict_mask(&expr.source_id, col_offset, &first.schema())
                            } else {
                                vec![]
                            };
                            let encoded: Vec<RecordBatch> = if dict_mask.iter().any(|&b| b) {
                                projected
                                    .iter()
                                    .map(|b| maybe_dict_encode_batch(b, &dict_mask))
                                    .collect::<Result<_, _>>()?
                            } else {
                                projected
                            };
                            let data = serialize_to_arrow_ipc(&encoded)?;
                            return Ok(TileResponse {
                                source_id: expr.source_id.clone(),
                                row,
                                col: col_offset,
                                data,
                                is_provisional: false,
                                job_id: None,
                            });
                        }
                    }
                }

                let cache_key = format!("{}:sl:{}", expr.source_id, norm_hash);
                let meta_c = meta.clone();
                let ops_c = normalized.clone();
                let schema_hint = self.schema_cache.read().unwrap().get(&meta.id).cloned();
                let mat_view = self
                    .materializer
                    .get_or_materialize(cache_key, move || async move {
                        let all_batches = read_with_pushdown(&meta_c, &ops_c, schema_hint).await?;
                        let processed =
                            executor::execute_pipeline_skip_filter(all_batches, &ops_c)?;
                        let total_rows: u64 = processed.iter().map(|b| b.num_rows() as u64).sum();
                        Ok(MaterializedView::Batches {
                            batches: processed,
                            total_rows,
                        })
                    })
                    .await?;

                if let MaterializedView::Batches { batches, .. } = mat_view.as_ref() {
                    let total: usize = batches.iter().map(|b| b.num_rows()).sum();
                    let start = (row as usize).min(total);
                    let len = (rows as usize).min(total.saturating_sub(start));
                    let sliced = slice_batches(batches, start, len);
                    let projected: Vec<RecordBatch> = sliced
                        .iter()
                        .map(|b| project_tile_columns(b, col_offset, cols))
                        .collect::<Result<_, _>>()?;
                    let dict_mask = if let Some(first) = projected.first() {
                        self.build_dict_mask(&expr.source_id, col_offset, &first.schema())
                    } else {
                        vec![]
                    };
                    let encoded: Vec<RecordBatch> = if dict_mask.iter().any(|&b| b) {
                        projected
                            .iter()
                            .map(|b| maybe_dict_encode_batch(b, &dict_mask))
                            .collect::<Result<_, _>>()?
                    } else {
                        projected
                    };
                    let data = serialize_to_arrow_ipc(&encoded)?;
                    return Ok(TileResponse {
                        source_id: expr.source_id.clone(),
                        row,
                        col: col_offset,
                        data,
                        is_provisional: false,
                        job_id: None,
                    });
                }

                Err(EngineError::Query(
                    "unexpected materialized view type".into(),
                ))
            }

            PipelineClass::NeedsMaterialization => {
                if is_sort_satisfied_by_presort(&normalized, &meta) {
                    let col_end = (col_offset + cols).min(meta.n_cols);
                    let col_indices: Vec<usize> = (col_offset..col_end).collect();
                    let presort_metadata_arc = if matches!(meta.format, SourceFormat::Parquet)
                        && !is_cloud_kind(&meta.kind)
                        && meta.files.len() <= 1
                    {
                        let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
                        self.metadata_cache.read().unwrap().get(path).cloned()
                    } else {
                        None
                    };
                    let batches = read_tile_dispatch(
                        &meta,
                        row as usize,
                        &col_indices,
                        rows as usize,
                        presort_metadata_arc,
                    )
                    .await?;
                    let dict_mask = if let Some(first) = batches.first() {
                        self.build_dict_mask(&expr.source_id, col_offset, &first.schema())
                    } else {
                        vec![]
                    };
                    let encoded: Vec<RecordBatch> = if dict_mask.iter().any(|&b| b) {
                        batches
                            .iter()
                            .map(|b| maybe_dict_encode_batch(b, &dict_mask))
                            .collect::<Result<_, _>>()?
                    } else {
                        batches
                    };
                    let data = serialize_to_arrow_ipc(&encoded)?;
                    return Ok(TileResponse {
                        source_id: expr.source_id.clone(),
                        row,
                        col: col_offset,
                        data,
                        is_provisional: false,
                        job_id: None,
                    });
                }

                let cache_key = format!("{}:{}", expr.source_id, norm_hash);
                let meta_c = meta.clone();
                let ops_c = normalized.clone();
                let schema_hint = self.schema_cache.read().unwrap().get(&meta.id).cloned();
                let temp_root = self.temp_root.clone();

                let mat_view = self
                    .materializer
                    .get_or_materialize(cache_key.clone(), move || async move {
                        tokio::task::spawn_blocking(move || {
                            SpillPipeline::new(temp_root).build(&meta_c, &ops_c, schema_hint)
                        })
                        .await
                        .map_err(|e| EngineError::Query(e.to_string()))?
                    })
                    .await?;

                if mat_view.is_provisional() {
                    let cache_key_bg = cache_key.clone();
                    let meta_bg = meta.clone();
                    let ops_bg = normalized.clone();
                    let schema_hint_bg = self.schema_cache.read().unwrap().get(&meta.id).cloned();
                    let temp_root_bg = self.temp_root.clone();
                    let materializer_bg = Arc::clone(&self.materializer);
                    let job_id_bg = mat_view.job_id().unwrap_or("").to_string();
                    let job_registry_bg = Arc::clone(&self.job_registry);
                    let view_expr_bg = expr.clone();
                    let view_hash_bg = norm_hash.clone();
                    tokio::spawn(async move {
                        if let Ok(Ok(full_view)) = tokio::task::spawn_blocking(move || {
                            SpillPipeline::new(temp_root_bg).build_full(
                                &meta_bg,
                                &ops_bg,
                                schema_hint_bg,
                            )
                        })
                        .await
                        {
                            let total = full_view.row_count();
                            materializer_bg.replace(cache_key_bg, full_view).await;
                            if !job_id_bg.is_empty() {
                                let job =
                                    job_registry_bg.create_job(view_expr_bg, view_hash_bg).await;
                                job.emit(crate::job_registry::JobEvent::Complete {
                                    total_rows: total,
                                    elapsed_ms: 0,
                                })
                                .await;
                            }
                        }
                    });
                }

                let dict_mask_nm = {
                    let schema_hint = self.schema_cache.read().unwrap().get(&meta.id).cloned();
                    if let Some(schema) = schema_hint {
                        let col_end = (col_offset + cols).min(schema.fields().len());
                        let projected_fields: Vec<_> =
                            schema.fields()[col_offset..col_end].to_vec();
                        let projected_schema =
                            Arc::new(arrow::datatypes::Schema::new(projected_fields));
                        self.build_dict_mask(&expr.source_id, col_offset, &projected_schema)
                    } else {
                        vec![]
                    }
                };
                serve_materialized_tile(
                    mat_view.as_ref(),
                    row,
                    col_offset,
                    rows,
                    cols,
                    &expr.source_id,
                    &dict_mask_nm,
                )
            }
        }
    }

    pub async fn query_view_tile_agg(
        &self,
        expr: &ViewExpr,
        row: u64,
        col_offset: usize,
        rows: u64,
        cols: usize,
    ) -> Result<TileResponse, EngineError> {
        let raw = self
            .query_view_tile(expr, row, col_offset, rows, cols)
            .await?;
        let raw_batches = deserialize_ipc_to_batches(&raw.data)?;
        let agg_batch = aggregate_tile_batch(&raw_batches)?;
        let agg_data = query::serialize_to_arrow_ipc(&[agg_batch])?;
        Ok(TileResponse {
            source_id: raw.source_id,
            row: raw.row,
            col: raw.col,
            data: agg_data,
            is_provisional: raw.is_provisional,
            job_id: raw.job_id,
        })
    }

    pub async fn query_view_tile_batch(
        &self,
        expr: &ViewExpr,
        tiles: &[BatchTileRequest],
    ) -> Result<Vec<TileResponse>, EngineError> {
        if tiles.is_empty() {
            return Ok(vec![]);
        }
        let futs: Vec<_> = tiles
            .iter()
            .map(|t| self.query_view_tile(expr, t.row, t.col, t.rows, t.cols))
            .collect();
        futures::future::join_all(futs).await.into_iter().collect()
    }

    pub async fn query_view_count(&self, expr: &ViewExpr) -> Result<u64, EngineError> {
        if let Some(true) = self.check_source_stale(&expr.source_id) {
            return Err(EngineError::Query("source_modified".into()));
        }
        let meta = self
            .catalog
            .get(&expr.source_id)
            .ok_or_else(|| EngineError::SourceNotFound(expr.source_id.clone()))?;

        let stats_for_opt: Vec<ColumnStats> = {
            let cache = self.stats_cache.read().unwrap();
            let id = &meta.id;
            let mut by_col: Vec<Option<ColumnStats>> = vec![None; meta.n_cols];
            for ((sid, col_idx), stats) in cache.iter() {
                if sid == id && *col_idx < meta.n_cols {
                    by_col[*col_idx] = Some(stats.clone());
                }
            }
            by_col.into_iter().flatten().collect()
        };
        let stats_hint = if stats_for_opt.is_empty() {
            None
        } else {
            Some(stats_for_opt.as_slice())
        };
        let source_path_for_q = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
        let quantile_arc = self
            .quantile_cache
            .read()
            .unwrap()
            .get(source_path_for_q)
            .cloned();
        let optimized = optimize_with_quantiles(&expr.ops, stats_hint, quantile_arc.as_deref());
        let normalized = tv_core::normalize_ops(&optimized);
        let norm_hash = view_hash(&format!("{:?}", normalized));

        let count_changes = normalized.iter().any(|op| {
            matches!(
                op,
                ViewOp::Filter { .. }
                    | ViewOp::Deduplicate { .. }
                    | ViewOp::Sample { .. }
                    | ViewOp::GroupBy { .. }
                    | ViewOp::Limit { .. }
                    | ViewOp::TopK { .. }
            )
        });

        if !count_changes {
            return Ok(meta.n_rows);
        }

        let sort_only = normalized
            .iter()
            .all(|op| matches!(op, ViewOp::Sort { .. }));
        if sort_only {
            return Ok(meta.n_rows);
        }

        if let Some(ViewOp::TopK { n, .. }) = normalized
            .iter()
            .find(|op| matches!(op, ViewOp::TopK { .. }))
        {
            let only_topk_and_stateless = normalized.iter().all(|op| {
                matches!(
                    op,
                    ViewOp::TopK { .. }
                        | ViewOp::Sort { .. }
                        | ViewOp::Select { .. }
                        | ViewOp::Drop { .. }
                        | ViewOp::Derive { .. }
                        | ViewOp::Rename { .. }
                )
            });
            if only_topk_and_stateless {
                return Ok((*n).min(meta.n_rows));
            }
        }

        let only_filter = normalized.iter().all(|op| {
            matches!(
                op,
                ViewOp::Filter { .. }
                    | ViewOp::Sort { .. }
                    | ViewOp::Select { .. }
                    | ViewOp::Drop { .. }
                    | ViewOp::Rename { .. }
            )
        });
        if only_filter && !meta.quick_stats.is_empty() {
            if let Some(pred) = extract_combined_filter(&normalized) {
                let fast_count = match &pred {
                    tv_core::Predicate::IsNull { column } => meta
                        .columns
                        .iter()
                        .position(|c| &c.name == column)
                        .and_then(|idx| meta.quick_stats.get(idx))
                        .map(|qs| qs.null_count),
                    tv_core::Predicate::IsNotNull { column } => meta
                        .columns
                        .iter()
                        .position(|c| &c.name == column)
                        .and_then(|idx| meta.quick_stats.get(idx))
                        .map(|qs| meta.n_rows.saturating_sub(qs.null_count)),
                    _ => None,
                };
                if let Some(c) = fast_count {
                    return Ok(c);
                }
            }
        }

        let cache_key = format!("{}:{}", expr.source_id, norm_hash);
        if let Some(mat_view) = self.materializer.get(&cache_key).await {
            return Ok(mat_view.row_count());
        }

        match classify_pipeline(&normalized) {
            PipelineClass::PureRead => Ok(meta.n_rows),

            PipelineClass::StatelessOnly => {
                if matches!(meta.format, SourceFormat::Parquet) && !is_cloud_kind(&meta.kind) {
                    if let Some(pred) = extract_combined_filter(&normalized) {
                        let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
                        let schema = match self.schema_cache.read().unwrap().get(&meta.id).cloned()
                        {
                            Some(s) => s,
                            None => reader::parquet_schema_and_rows(path).map(|(s, _)| s)?,
                        };
                        if meta.files.len() <= 1 {
                            let bloom_arc = self.bloom_cache.read().unwrap().get(path).cloned();
                            let metadata_arc =
                                self.metadata_cache.read().unwrap().get(path).cloned();
                            let roaring_arc =
                                if let Some(pred_col) = pred_col_for_roaring(&pred, &schema) {
                                    self.roaring_cache
                                        .read()
                                        .unwrap()
                                        .get(&(path.to_string(), pred_col))
                                        .cloned()
                                } else {
                                    None
                                };
                            let mark_rgs =
                                mark_qualifying_rgs(&pred, &schema, path, &self.mark_cache);
                            return reader::count_parquet_filtered(
                                path,
                                &pred,
                                &schema,
                                bloom_arc.as_deref(),
                                roaring_arc.as_deref(),
                                metadata_arc,
                                mark_rgs.as_deref(),
                            );
                        } else {
                            use rayon::prelude::*;
                            let total: u64 = meta
                                .files
                                .par_iter()
                                .map(|fp| {
                                    reader::count_parquet_filtered(
                                        fp, &pred, &schema, None, None, None, None,
                                    )
                                })
                                .collect::<Result<Vec<_>, _>>()?
                                .into_iter()
                                .sum();
                            return Ok(total);
                        }
                    }
                }
                let sl_cache_key = format!("{}:sl:{}", expr.source_id, norm_hash);
                if let Some(mat_view) = self.materializer.get(&sl_cache_key).await {
                    return Ok(mat_view.row_count());
                }
                let meta_c = meta.clone();
                let ops_c = normalized.clone();
                let schema_hint = self.schema_cache.read().unwrap().get(&meta.id).cloned();
                let mat_view = self
                    .materializer
                    .get_or_materialize(sl_cache_key, move || async move {
                        let all_batches = read_with_pushdown(&meta_c, &ops_c, schema_hint).await?;
                        let processed =
                            executor::execute_pipeline_skip_filter(all_batches, &ops_c)?;
                        let total_rows: u64 = processed.iter().map(|b| b.num_rows() as u64).sum();
                        Ok(MaterializedView::Batches {
                            batches: processed,
                            total_rows,
                        })
                    })
                    .await?;
                Ok(mat_view.row_count())
            }

            PipelineClass::NeedsMaterialization => {
                let meta_c = meta.clone();
                let ops_c = normalized.clone();
                let schema_hint = self.schema_cache.read().unwrap().get(&meta.id).cloned();
                let temp_root = self.temp_root.clone();
                let mat_view = self
                    .materializer
                    .get_or_materialize(cache_key, move || async move {
                        tokio::task::spawn_blocking(move || {
                            SpillPipeline::new(temp_root).build(&meta_c, &ops_c, schema_hint)
                        })
                        .await
                        .map_err(|e| EngineError::Query(e.to_string()))?
                    })
                    .await?;
                Ok(mat_view.row_count())
            }
        }
    }

    pub fn query_view_schema(&self, expr: &ViewExpr) -> Result<Vec<ColumnInfo>, EngineError> {
        if let Some(true) = self.check_source_stale(&expr.source_id) {
            return Err(EngineError::Query("source_modified".into()));
        }
        let meta = self
            .catalog
            .get(&expr.source_id)
            .ok_or_else(|| EngineError::SourceNotFound(expr.source_id.clone()))?;

        let normalized = tv_core::normalize_ops(&expr.ops);
        let schema = compiler::schema::infer_schema(&meta.columns, &normalized)
            .map_err(|e| EngineError::Query(e.to_string()))?;

        Ok(schema)
    }

    pub fn codegen(&self, expr: &ViewExpr, target: CodegenTarget) -> Result<String, EngineError> {
        let meta = self
            .catalog
            .get(&expr.source_id)
            .ok_or_else(|| EngineError::SourceNotFound(expr.source_id.clone()))?;
        Ok(match target {
            CodegenTarget::DuckdbSql => export::sql::render_sql(expr, &meta.uri, &meta.format),
            CodegenTarget::AnsiSql => export::ansi_sql::render_ansi_sql(expr, &meta.name),
            CodegenTarget::PythonPandas => export::python::render_python(
                expr,
                &meta.uri,
                &meta.format,
                export::python::PythonDialect::Pandas,
            ),
            CodegenTarget::PythonPolars => export::python::render_python(
                expr,
                &meta.uri,
                &meta.format,
                export::python::PythonDialect::Polars,
            ),
            CodegenTarget::PythonDuckdb => export::python::render_python(
                expr,
                &meta.uri,
                &meta.format,
                export::python::PythonDialect::DuckDb,
            ),
            CodegenTarget::Shell => export::shell::render_shell(expr, &meta.uri, &meta.format),
            CodegenTarget::ShellCsv => {
                export::shell::render_shell_csv(expr, &meta.uri, &meta.format)
            }
            CodegenTarget::Dbt => export::dbt::render_dbt(expr, &meta.name),
        })
    }

    pub async fn download_view(
        &self,
        expr: &ViewExpr,
        format: DownloadFormat,
    ) -> Result<(Vec<u8>, &'static str), EngineError> {
        let meta = self
            .catalog
            .get(&expr.source_id)
            .ok_or_else(|| EngineError::SourceNotFound(expr.source_id.clone()))?;

        let normalized = tv_core::normalize_ops(&expr.ops);

        let batches = match classify_pipeline(&normalized) {
            PipelineClass::NeedsMaterialization => {
                let cache_key = format!(
                    "{}:{}",
                    expr.source_id,
                    view_hash(&format!("{:?}", normalized))
                );
                let schema_hint = self.schema_cache.read().unwrap().get(&meta.id).cloned();
                let mat_view = if let Some(mv) = self.materializer.get(&cache_key).await {
                    mv
                } else {
                    let meta_c = meta.clone();
                    let ops_c = normalized.clone();
                    let temp_root = self.temp_root.clone();
                    self.materializer
                        .get_or_materialize(cache_key, move || async move {
                            let pipeline = SpillPipeline::new(temp_root);
                            pipeline.build(&meta_c, &ops_c, schema_hint)
                        })
                        .await?
                };
                collect_materialized_view(mat_view.as_ref())?
            }
            _ => {
                let schema_hint = self.schema_cache.read().unwrap().get(&meta.id).cloned();
                let batches = read_with_pushdown(&meta, &normalized, schema_hint).await?;
                executor::execute_pipeline_skip_filter(batches, &normalized)?
            }
        };

        let data = match format {
            DownloadFormat::Arrow => serialize_to_arrow_ipc(&batches)?,
            DownloadFormat::Parquet => {
                let schema = if batches.is_empty() {
                    Arc::new(arrow::datatypes::Schema::empty())
                } else {
                    batches[0].schema()
                };
                let tmp = std::env::temp_dir().join(format!("tv_{}.parquet", uuid::Uuid::new_v4()));
                let file = std::fs::File::create(&tmp)?;
                let mut writer = parquet::arrow::ArrowWriter::try_new(file, schema, None)?;
                for batch in &batches {
                    writer.write(batch)?;
                }
                writer.close()?;
                let bytes = std::fs::read(&tmp)?;
                let _ = std::fs::remove_file(&tmp);
                bytes
            }
            DownloadFormat::Csv => {
                let mut buf = Vec::new();
                {
                    let mut writer = arrow_csv::WriterBuilder::new()
                        .with_header(true)
                        .build(&mut buf);
                    for batch in &batches {
                        writer.write(batch)?;
                    }
                }
                buf
            }
            DownloadFormat::Jsonl => write_jsonl(&batches),
        };

        Ok((data, format.content_type()))
    }

    fn build_dict_mask(
        &self,
        source_id: &str,
        col_offset: usize,
        projected_batch_schema: &arrow::datatypes::Schema,
    ) -> Vec<bool> {
        let stats_cache = self.stats_cache.read().unwrap();
        projected_batch_schema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let source_col = col_offset + i;
                if !matches!(
                    field.data_type(),
                    arrow::datatypes::DataType::Utf8 | arrow::datatypes::DataType::LargeUtf8
                ) {
                    return false;
                }
                let key = (source_id.to_string(), source_col);
                stats_cache
                    .get(&key)
                    .and_then(|s| s.distinct_count)
                    .map(|dc| dc < 1000)
                    .unwrap_or(false)
            })
            .collect()
    }

    pub async fn column_stats(
        &self,
        source_id: &str,
        col_idx: usize,
        n_bins: usize,
    ) -> Result<ColumnStats, EngineError> {
        if n_bins == stats::DEFAULT_HISTOGRAM_BINS {
            let cache = self.stats_cache.read().unwrap();
            if let Some(cached) = cache.get(&(source_id.to_string(), col_idx)) {
                return Ok(cached.clone());
            }
        }
        let meta = self
            .catalog
            .get(source_id)
            .ok_or_else(|| EngineError::SourceNotFound(source_id.to_string()))?;
        debug!(source = %source_id, col = col_idx, "computing column stats");
        let result = stats::compute_column_stats(&meta, col_idx, n_bins)?;
        if n_bins == stats::DEFAULT_HISTOGRAM_BINS {
            self.stats_cache
                .write()
                .unwrap()
                .insert((source_id.to_string(), col_idx), result.clone());
        }
        Ok(result)
    }

    pub async fn column_stats_coarse(
        &self,
        source_id: &str,
        col_idx: usize,
    ) -> Result<ColumnStats, EngineError> {
        let meta = self
            .catalog
            .get(source_id)
            .ok_or_else(|| EngineError::SourceNotFound(source_id.to_string()))?;

        if !matches!(meta.format, tv_core::SourceFormat::Parquet)
            || meta.kind != tv_core::SourceKind::LocalFile
        {
            return Err(EngineError::Query(
                "coarse stats only supported for local Parquet sources".into(),
            ));
        }

        let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);

        let first_rg_rows = {
            let mc = self.metadata_cache.read().unwrap();
            mc.get(path)
                .map(|m| m.row_group(0).num_rows() as usize)
                .unwrap_or(50000)
        };

        let coarse_meta_arc = self.metadata_cache.read().unwrap().get(path).cloned();
        let batches =
            reader::read_parquet_tile(path, 0, &[col_idx], first_rg_rows, coarse_meta_arc)?;

        let col = meta
            .columns
            .get(col_idx)
            .ok_or_else(|| EngineError::SourceNotFound(format!("column index {col_idx}")))?;

        stats::compute_column_stats_from_batches(col, col_idx, &batches, 10)
    }

    pub async fn row_group_stats(
        &self,
        source_id: &str,
        col_idx: usize,
    ) -> Result<Vec<reader::RowGroupColumnStat>, EngineError> {
        let meta = self
            .catalog
            .get(source_id)
            .ok_or_else(|| EngineError::SourceNotFound(source_id.to_string()))?;

        if !matches!(meta.format, tv_core::SourceFormat::Parquet)
            || meta.kind != tv_core::SourceKind::LocalFile
        {
            return Ok(vec![]);
        }

        let path = if !meta.files.is_empty() {
            meta.files[0].clone()
        } else {
            meta.uri.clone()
        };

        reader::parquet_row_group_column_stats(&path, col_idx)
    }

    pub async fn correlations(&self, source_id: &str) -> Result<CorrelationMatrix, EngineError> {
        let meta = self
            .catalog
            .get(source_id)
            .ok_or_else(|| EngineError::SourceNotFound(source_id.to_string()))?;
        stats::compute_correlations(&meta)
    }

    pub async fn search(
        &self,
        source_id: &str,
        query: &str,
        columns: Option<Vec<String>>,
        limit: usize,
    ) -> Result<Vec<u64>, EngineError> {
        let meta = self
            .catalog
            .get(source_id)
            .ok_or_else(|| EngineError::SourceNotFound(source_id.to_string()))?;

        let target_col_names: Vec<String> = match columns {
            Some(c) if !c.is_empty() => c,
            _ => meta.columns.iter().map(|c| c.name.clone()).collect(),
        };

        let target_col_indices: Vec<usize> = target_col_names
            .iter()
            .filter_map(|name| {
                meta.columns
                    .iter()
                    .find(|c| &c.name == name)
                    .map(|c| c.index)
            })
            .collect();

        let batches = read_full_dispatch(&meta, Some(&target_col_indices)).await?;

        let mut results = Vec::new();
        let mut global_row = 0u64;

        'outer: for batch in &batches {
            let schema = batch.schema();
            let col_indices: Vec<usize> = target_col_names
                .iter()
                .filter_map(|name| schema.index_of(name).ok())
                .collect();

            for row_idx in 0..batch.num_rows() {
                for &col_idx in &col_indices {
                    let col = batch.column(col_idx);
                    if col.is_null(row_idx) {
                        continue;
                    }
                    let cell =
                        match arrow::compute::cast(col.as_ref(), &arrow::datatypes::DataType::Utf8)
                        {
                            Ok(str_col) => {
                                if let Some(s) =
                                    str_col.as_any().downcast_ref::<arrow::array::StringArray>()
                                {
                                    if s.is_null(row_idx) {
                                        continue;
                                    }
                                    s.value(row_idx).to_string()
                                } else {
                                    continue;
                                }
                            }
                            Err(_) => continue,
                        };
                    if cell.contains(query) {
                        results.push(global_row + row_idx as u64);
                        break;
                    }
                }
                if results.len() >= limit {
                    break 'outer;
                }
            }
            global_row += batch.num_rows() as u64;
        }

        Ok(results)
    }

    pub fn ops_view_hash(expr: &ViewExpr) -> String {
        view_hash(&format!("{:?}", expr.ops))
    }

    pub async fn inspect_uri(
        &self,
        uri: &str,
        profile: Option<String>,
        credentials: Option<Credentials>,
    ) -> Result<serde_json::Value, EngineError> {
        let meta = self
            .register_source(uri, None, profile, credentials)
            .await?;
        let expr = ViewExpr {
            source_id: meta.id.clone(),
            ops: vec![],
        };
        let count = self.query_view_count(&expr).await?;
        let schema = self.query_view_schema(&expr)?;
        let _ = self.remove_source(&meta.id).await;
        Ok(serde_json::json!({
            "uri": uri,
            "n_rows": count,
            "n_cols": schema.len(),
            "columns": schema
        }))
    }

    pub async fn profile_source(&self, source_id: &str) -> Result<Vec<ColumnStats>, EngineError> {
        let meta = self
            .get_source(source_id)
            .ok_or_else(|| EngineError::SourceNotFound(source_id.to_string()))?;
        info!(source = %source_id, n_cols = meta.n_cols, "profiling source");
        let t0 = std::time::Instant::now();
        use rayon::prelude::*;
        let all_stats: Result<Vec<_>, _> = (0..meta.n_cols)
            .into_par_iter()
            .map(|i| stats::compute_column_stats(&meta, i, stats::DEFAULT_HISTOGRAM_BINS))
            .collect();
        let all_stats = all_stats?;
        {
            let mut cache = self.stats_cache.write().unwrap();
            for (i, s) in all_stats.iter().enumerate() {
                cache.insert((source_id.to_string(), i), s.clone());
            }
        }
        info!(source = %source_id, elapsed_ms = t0.elapsed().as_millis(), "profile complete");
        Ok(all_stats)
    }

    pub async fn optimize_source(&self, source_id: &str) -> Result<(), EngineError> {
        let meta = self
            .catalog
            .get(source_id)
            .ok_or_else(|| EngineError::SourceNotFound(source_id.to_string()))?;

        if !matches!(meta.format, SourceFormat::Parquet)
            || is_cloud_kind(&meta.kind)
            || meta.files.len() != 1
        {
            return Err(EngineError::Query(
                "optimize only supported for single-file local Parquet sources".into(),
            ));
        }

        let source_path = meta.files[0].clone();
        let source_id_owned = source_id.to_string();
        let _engine = self.clone();

        tokio::task::spawn_blocking(move || -> Result<(), EngineError> {
            let batches = reader::read_parquet_full(&source_path, None)?;
            if batches.is_empty() {
                return Ok(());
            }

            let schema = batches[0].schema();
            let total_bytes: i64 = batches
                .iter()
                .map(|b| b.get_array_memory_size() as i64)
                .sum();
            let total_rows: i64 = batches.iter().map(|b| b.num_rows() as i64).sum();
            let bytes_per_row = if total_rows > 0 {
                (total_bytes / total_rows).max(1)
            } else {
                64
            };
            let target_rg_rows =
                ((512 * 1024 * 1024) / bytes_per_row).clamp(4096, 4_000_000) as usize;

            let props = parquet::file::properties::WriterProperties::builder()
                .set_writer_version(parquet::file::properties::WriterVersion::PARQUET_2_0)
                .set_compression(parquet::basic::Compression::SNAPPY)
                .set_data_page_size_limit(65536)
                .set_max_row_group_size(target_rg_rows)
                .set_statistics_enabled(parquet::file::properties::EnabledStatistics::Chunk)
                .build();

            let tmp_path = format!("{source_path}.optimize_tmp");
            let file = std::fs::File::create(&tmp_path)?;
            let mut writer = parquet::arrow::ArrowWriter::try_new(file, schema, Some(props))?;

            for batch in &batches {
                writer.write(batch)?;
            }
            writer.close()?;
            std::fs::rename(&tmp_path, &source_path)?;
            Ok(())
        })
        .await
        .map_err(|e| EngineError::Internal(e.to_string()))??;

        self.metadata_cache.write().unwrap().remove(&meta.files[0]);

        let uri = meta.uri.clone();
        let name = Some(meta.name.clone());
        self.remove_source(&source_id_owned).await?;
        self.register_source(&uri, name, None, None).await?;
        Ok(())
    }
}

fn serve_materialized_tile(
    mat_view: &MaterializedView,
    row: u64,
    col_offset: usize,
    rows: u64,
    cols: usize,
    source_id: &str,
    dict_mask: &[bool],
) -> Result<TileResponse, EngineError> {
    match mat_view {
        MaterializedView::Batches { batches, .. } => {
            let total: usize = batches.iter().map(|b| b.num_rows()).sum();
            let start = (row as usize).min(total);
            let len = (rows as usize).min(total.saturating_sub(start));
            let sliced = slice_batches(batches, start, len);
            let projected: Vec<RecordBatch> = sliced
                .iter()
                .map(|b| project_tile_columns(b, col_offset, cols))
                .collect::<Result<_, _>>()?;
            let encoded: Vec<RecordBatch> = if dict_mask.iter().any(|&b| b) {
                projected
                    .iter()
                    .map(|b| maybe_dict_encode_batch(b, dict_mask))
                    .collect::<Result<_, _>>()?
            } else {
                projected
            };
            let data = serialize_to_arrow_ipc(&encoded)?;
            Ok(TileResponse {
                source_id: source_id.to_string(),
                row,
                col: col_offset,
                data,
                is_provisional: false,
                job_id: None,
            })
        }
        MaterializedView::SortedRuns {
            runs,
            cumulative_rows,
            schema,
            sort_keys,
            dedup_columns,
            ..
        } => {
            let sorter = ExternalSorter::new(sort_keys.clone(), schema.clone());
            let batches = match dedup_columns {
                None => sorter.merge_tile(runs, cumulative_rows, row as usize, rows as usize)?,
                Some(dedup_cols) => sorter.merge_dedup_tile(
                    runs,
                    cumulative_rows,
                    dedup_cols,
                    row as usize,
                    rows as usize,
                )?,
            };
            let projected: Vec<RecordBatch> = batches
                .iter()
                .map(|b| project_tile_columns(b, col_offset, cols))
                .collect::<Result<_, _>>()?;
            let encoded: Vec<RecordBatch> = if dict_mask.iter().any(|&b| b) {
                projected
                    .iter()
                    .map(|b| maybe_dict_encode_batch(b, dict_mask))
                    .collect::<Result<_, _>>()?
            } else {
                projected
            };
            let data = serialize_to_arrow_ipc(&encoded)?;
            Ok(TileResponse {
                source_id: source_id.to_string(),
                row,
                col: col_offset,
                data,
                is_provisional: false,
                job_id: None,
            })
        }
        MaterializedView::AggregateResult { run, schema, .. } => {
            let n_cols = schema.fields().len();
            let col_end = (col_offset + cols).min(n_cols);
            let col_indices: Vec<usize> = (col_offset..col_end).collect();
            let batches = crate::reader::read_parquet_tile(
                run.path.to_str().unwrap_or(""),
                row as usize,
                &col_indices,
                rows as usize,
                None,
            )?;
            let data = serialize_to_arrow_ipc(&batches)?;
            Ok(TileResponse {
                source_id: source_id.to_string(),
                row,
                col: col_offset,
                data,
                is_provisional: false,
                job_id: None,
            })
        }
        MaterializedView::SortIndexBacked {
            index_path,
            source_path,
            n_cols,
            ..
        } => {
            let row_ids = crate::sort_index::tile_lookup(index_path, row as usize, rows as usize)?;
            let col_end = (col_offset + cols).min(*n_cols);
            let col_indices: Vec<usize> = (col_offset..col_end).collect();
            let batches = crate::sort_index::read_rows_by_ids(source_path, &row_ids, &col_indices)?;
            let data = serialize_to_arrow_ipc(&batches)?;
            Ok(TileResponse {
                source_id: source_id.to_string(),
                row,
                col: col_offset,
                data,
                is_provisional: false,
                job_id: None,
            })
        }
        MaterializedView::ProvisionalAgg {
            batches, job_id, ..
        } => {
            let total: usize = batches.iter().map(|b| b.num_rows()).sum();
            let start = (row as usize).min(total);
            let len = (rows as usize).min(total.saturating_sub(start));
            let sliced = slice_batches(batches, start, len);
            let projected: Vec<RecordBatch> = sliced
                .iter()
                .map(|b| project_tile_columns(b, col_offset, cols))
                .collect::<Result<_, _>>()?;
            let encoded: Vec<RecordBatch> = if dict_mask.iter().any(|&b| b) {
                projected
                    .iter()
                    .map(|b| maybe_dict_encode_batch(b, dict_mask))
                    .collect::<Result<_, _>>()?
            } else {
                projected
            };
            let data = serialize_to_arrow_ipc(&encoded)?;
            Ok(TileResponse {
                source_id: source_id.to_string(),
                row,
                col: col_offset,
                data,
                is_provisional: true,
                job_id: Some(job_id.clone()),
            })
        }
        MaterializedView::ProvisionalSort {
            runs,
            cumulative_rows,
            schema,
            sort_keys,
            job_id,
            ..
        } => {
            let sorter = ExternalSorter::new(sort_keys.clone(), schema.clone());
            let batches = sorter.merge_tile(runs, cumulative_rows, row as usize, rows as usize)?;
            let projected: Vec<RecordBatch> = batches
                .iter()
                .map(|b| project_tile_columns(b, col_offset, cols))
                .collect::<Result<_, _>>()?;
            let encoded: Vec<RecordBatch> = if dict_mask.iter().any(|&b| b) {
                projected
                    .iter()
                    .map(|b| maybe_dict_encode_batch(b, dict_mask))
                    .collect::<Result<_, _>>()?
            } else {
                projected
            };
            let data = serialize_to_arrow_ipc(&encoded)?;
            Ok(TileResponse {
                source_id: source_id.to_string(),
                row,
                col: col_offset,
                data,
                is_provisional: true,
                job_id: Some(job_id.clone()),
            })
        }
        MaterializedView::BitmapGroupBy { batches, .. } => {
            let total: usize = batches.iter().map(|b| b.num_rows()).sum();
            let start = (row as usize).min(total);
            let len = (rows as usize).min(total.saturating_sub(start));
            let sliced = slice_batches(batches, start, len);
            let projected: Vec<RecordBatch> = sliced
                .iter()
                .map(|b| project_tile_columns(b, col_offset, cols))
                .collect::<Result<_, _>>()?;
            let encoded: Vec<RecordBatch> = if dict_mask.iter().any(|&b| b) {
                projected
                    .iter()
                    .map(|b| maybe_dict_encode_batch(b, dict_mask))
                    .collect::<Result<_, _>>()?
            } else {
                projected
            };
            let data = serialize_to_arrow_ipc(&encoded)?;
            Ok(TileResponse {
                source_id: source_id.to_string(),
                row,
                col: col_offset,
                data,
                is_provisional: false,
                job_id: None,
            })
        }
        MaterializedView::SparseSortIndexBacked {
            index,
            spill_path,
            schema,
            ..
        } => {
            let lookups = sparse_sort_index::sparse_tile_lookup(index, row as usize, rows as usize);
            let batches = sparse_sort_index::read_sparse_tile(
                spill_path, &lookups, col_offset, cols, schema,
            )?;
            let data = serialize_to_arrow_ipc(&batches)?;
            Ok(TileResponse {
                source_id: source_id.to_string(),
                row,
                col: col_offset,
                data,
                is_provisional: false,
                job_id: None,
            })
        }
        MaterializedView::RowCount { .. } => Err(EngineError::Query(
            "unexpected RowCount materialized view for tile".into(),
        )),
    }
}

fn collect_materialized_view(mat_view: &MaterializedView) -> Result<Vec<RecordBatch>, EngineError> {
    match mat_view {
        MaterializedView::Batches { batches, .. } => Ok(batches.clone()),
        MaterializedView::SortedRuns {
            runs,
            schema,
            sort_keys,
            dedup_columns,
            ..
        } => {
            let sorter = ExternalSorter::new(sort_keys.clone(), schema.clone());
            match dedup_columns {
                None => sorter.merge_all(runs),
                Some(dedup_cols) => sorter.merge_dedup_tile(runs, &[], dedup_cols, 0, usize::MAX),
            }
        }
        MaterializedView::AggregateResult { run, .. } => {
            crate::spill::SpillReader::open(&run.path).map(|r| r.collect::<Result<Vec<_>, _>>())?
        }
        MaterializedView::SortIndexBacked {
            index_path,
            source_path,
            n_cols,
            ..
        } => {
            let row_ids = crate::sort_index::tile_lookup(index_path, 0, usize::MAX)?;
            let col_indices: Vec<usize> = (0..*n_cols).collect();
            crate::sort_index::read_rows_by_ids(source_path, &row_ids, &col_indices)
        }
        MaterializedView::SparseSortIndexBacked {
            index,
            spill_path,
            schema,
            total_rows,
            ..
        } => {
            let lookups = sparse_sort_index::sparse_tile_lookup(index, 0, *total_rows as usize);
            sparse_sort_index::read_sparse_tile(
                spill_path,
                &lookups,
                0,
                schema.fields().len(),
                schema,
            )
        }
        MaterializedView::ProvisionalAgg { batches, .. }
        | MaterializedView::BitmapGroupBy { batches, .. } => Ok(batches.clone()),
        MaterializedView::ProvisionalSort {
            runs,
            schema,
            sort_keys,
            ..
        } => {
            let sorter = ExternalSorter::new(sort_keys.clone(), schema.clone());
            sorter.merge_all(runs)
        }
        MaterializedView::RowCount { .. } => Ok(vec![]),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipelineClass {
    PureRead,
    StatelessOnly,
    NeedsMaterialization,
}

fn classify_pipeline(ops: &[ViewOp]) -> PipelineClass {
    if ops.is_empty() {
        return PipelineClass::PureRead;
    }
    for op in ops {
        match op {
            ViewOp::Sort { .. }
            | ViewOp::GroupBy { .. }
            | ViewOp::Deduplicate { .. }
            | ViewOp::Sample { .. }
            | ViewOp::Limit { .. }
            | ViewOp::TopK { .. } => return PipelineClass::NeedsMaterialization,
            _ => {}
        }
    }
    PipelineClass::StatelessOnly
}

fn detect_pre_sorted_by(
    files: &[String],
    format: &SourceFormat,
    kind: &SourceKind,
    schema: &arrow::datatypes::SchemaRef,
) -> Option<Vec<tv_core::SortKey>> {
    if !matches!(format, SourceFormat::Parquet) || is_cloud_kind(kind) || files.len() != 1 {
        return None;
    }
    let path = files.first()?.as_str();
    let (_, _, pq_meta) = reader::parquet_schema_rows_and_metadata(path).ok()?;
    if pq_meta.num_row_groups() == 0 {
        return None;
    }
    let first_sorting = pq_meta.row_group(0).sorting_columns()?;
    if first_sorting.is_empty() {
        return None;
    }
    let consistent = (0..pq_meta.num_row_groups())
        .all(|i| pq_meta.row_group(i).sorting_columns() == Some(first_sorting));
    if !consistent {
        return None;
    }
    let sort_keys: Vec<tv_core::SortKey> = first_sorting
        .iter()
        .filter_map(|sc| {
            let col_idx = sc.column_idx as usize;
            schema.fields().get(col_idx).map(|f| tv_core::SortKey {
                column: f.name().clone(),
                descending: sc.descending,
                nulls_last: !sc.nulls_first,
            })
        })
        .collect();
    if sort_keys.is_empty() {
        None
    } else {
        Some(sort_keys)
    }
}

fn is_sort_satisfied_by_presort(normalized: &[ViewOp], meta: &SourceMeta) -> bool {
    let pre_sorted = match &meta.pre_sorted_by {
        Some(p) if !p.is_empty() => p,
        _ => return false,
    };
    let sort_keys = match normalized.iter().find_map(|op| {
        if let ViewOp::Sort { keys } = op {
            Some(keys)
        } else {
            None
        }
    }) {
        Some(k) => k,
        None => return false,
    };
    let non_sort_ops: Vec<&ViewOp> = normalized
        .iter()
        .filter(|op| !matches!(op, ViewOp::Sort { .. }))
        .collect();
    if !non_sort_ops.is_empty() {
        return false;
    }
    sort_keys.len() <= pre_sorted.len()
        && sort_keys
            .iter()
            .zip(pre_sorted.iter())
            .all(|(req, existing)| {
                req.column == existing.column
                    && req.descending == existing.descending
                    && req.nulls_last == existing.nulls_last
            })
}

async fn expand_source_files(uri: &str, kind: &SourceKind) -> Result<Vec<String>, EngineError> {
    match kind {
        SourceKind::LocalFile => {
            if uri.contains('*') || uri.contains('?') || uri.contains('[') {
                let files = reader::expand_local_glob(uri)?;
                if files.is_empty() {
                    return Err(EngineError::Query(format!(
                        "glob pattern matched no files: {uri}"
                    )));
                }
                Ok(files)
            } else {
                Ok(vec![uri.to_string()])
            }
        }
        SourceKind::S3 | SourceKind::Gcs | SourceKind::AzureBlob => {
            let parsed = url::Url::parse(uri).map_err(|e| EngineError::Query(e.to_string()))?;
            if parsed.path().ends_with('/') || parsed.path().is_empty() {
                reader::list_cloud_parquet_files(uri).await
            } else {
                Ok(vec![uri.to_string()])
            }
        }
        _ => Ok(vec![uri.to_string()]),
    }
}

async fn inspect_source(
    primary_uri: &str,
    format: &SourceFormat,
    files: &[String],
    kind: &SourceKind,
) -> Result<(Arc<arrow::datatypes::Schema>, u64), EngineError> {
    match format {
        SourceFormat::Parquet => {
            if is_cloud_kind(kind) {
                let (schema, first_rows) =
                    reader::parquet_schema_and_rows_cloud(primary_uri).await?;
                let n_rows = if files.len() <= 1 {
                    first_rows
                } else {
                    let mut total = first_rows;
                    for uri in files.iter().skip(1) {
                        let (_, r) = reader::parquet_schema_and_rows_cloud(uri).await?;
                        total += r;
                    }
                    total
                };
                Ok((schema, n_rows))
            } else {
                let (schema, first_rows) = reader::parquet_schema_and_rows(primary_uri)?;
                let n_rows = if files.len() <= 1 {
                    first_rows
                } else {
                    use rayon::prelude::*;
                    let extra: u64 = files[1..]
                        .par_iter()
                        .map(|path| reader::parquet_schema_and_rows(path).map(|(_, r)| r))
                        .collect::<Result<Vec<_>, _>>()?
                        .into_iter()
                        .sum();
                    first_rows + extra
                };
                Ok((schema, n_rows))
            }
        }
        SourceFormat::Csv => {
            let (schema, batches) = reader::csv_schema_and_data(primary_uri)?;
            let n_rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
            Ok((schema, n_rows))
        }
        SourceFormat::Json => {
            let (schema, batches) = reader::json_schema_and_data(primary_uri)?;
            let n_rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
            Ok((schema, n_rows))
        }
        SourceFormat::Arrow => {
            let (schema, batches) = reader::arrow_ipc_schema_and_data(primary_uri)?;
            let n_rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
            Ok((schema, n_rows))
        }
        other => Err(EngineError::UnsupportedFormat(format!("{other:?}"))),
    }
}

async fn read_tile_dispatch(
    meta: &SourceMeta,
    row_offset: usize,
    col_indices: &[usize],
    rows: usize,
    cached_metadata: Option<Arc<parquet::file::metadata::ParquetMetaData>>,
) -> Result<Vec<RecordBatch>, EngineError> {
    if is_cloud_kind(&meta.kind) {
        let uri = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
        return reader::read_cloud_parquet_tile(uri, row_offset, col_indices, rows).await;
    }

    let meta_c = meta.clone();
    let col_indices_c = col_indices.to_vec();
    tokio::task::spawn_blocking(move || match &meta_c.format {
        SourceFormat::Parquet => {
            if meta_c.files.len() <= 1 {
                let path = meta_c
                    .files
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or(&meta_c.uri);
                reader::read_parquet_tile(path, row_offset, &col_indices_c, rows, cached_metadata)
            } else {
                read_multi_file_tile(&meta_c.files, row_offset, &col_indices_c, rows)
            }
        }
        _ => {
            let uri = meta_c
                .files
                .first()
                .map(|s| s.as_str())
                .unwrap_or(&meta_c.uri);
            let all = reader::read_source_full(uri, &meta_c.format, Some(&col_indices_c))?;
            Ok(slice_batches(&all, row_offset, rows))
        }
    })
    .await
    .map_err(|e| EngineError::Query(e.to_string()))?
}

fn read_multi_file_tile(
    files: &[String],
    row_offset: usize,
    col_indices: &[usize],
    rows: usize,
) -> Result<Vec<RecordBatch>, EngineError> {
    let mut global_offset = 0usize;
    let mut collected: Vec<RecordBatch> = Vec::new();
    let mut remaining = rows;

    for path in files {
        if remaining == 0 {
            break;
        }
        let (_, file_rows) = reader::parquet_schema_and_rows(path)?;
        let file_rows = file_rows as usize;

        if global_offset + file_rows <= row_offset {
            global_offset += file_rows;
            continue;
        }

        let local_offset = row_offset.saturating_sub(global_offset);
        let batches = reader::read_parquet_tile(path, local_offset, col_indices, remaining, None)?;
        let got: usize = batches.iter().map(|b| b.num_rows()).sum();
        collected.extend(batches);
        remaining = remaining.saturating_sub(got);
        global_offset += file_rows;
    }

    Ok(collected)
}

async fn read_full_dispatch(
    meta: &SourceMeta,
    col_indices: Option<&[usize]>,
) -> Result<Vec<RecordBatch>, EngineError> {
    if is_cloud_kind(&meta.kind) {
        let uri = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
        return reader::read_cloud_parquet_full(uri, col_indices).await;
    }

    if meta.files.len() <= 1 {
        let uri = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
        reader::read_source_full(uri, &meta.format, col_indices)
    } else {
        use rayon::prelude::*;
        let all: Vec<RecordBatch> = meta
            .files
            .par_iter()
            .map(|path| reader::read_parquet_full(path, col_indices))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(all)
    }
}

async fn read_with_pushdown(
    meta: &SourceMeta,
    ops: &[ViewOp],
    schema_hint: Option<SchemaRef>,
) -> Result<Vec<RecordBatch>, EngineError> {
    let filter_pred = extract_combined_filter(ops);

    if let Some(pred) = filter_pred {
        if matches!(meta.format, SourceFormat::Parquet) && !is_cloud_kind(&meta.kind) {
            let schema = if let Some(s) = schema_hint {
                s
            } else {
                let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
                reader::parquet_schema_and_rows(path).map(|(s, _)| s)?
            };

            if meta.files.len() <= 1 {
                let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
                return reader::read_parquet_full_with_filter(path, &pred, &schema);
            } else {
                use rayon::prelude::*;
                let all: Vec<RecordBatch> = meta
                    .files
                    .par_iter()
                    .map(|path| reader::read_parquet_full_with_filter(path, &pred, &schema))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .flatten()
                    .collect();
                return Ok(all);
            }
        }
    }

    read_full_dispatch(meta, None).await
}

fn pred_col_for_roaring(
    predicate: &Predicate,
    schema: &arrow::datatypes::SchemaRef,
) -> Option<usize> {
    use crate::roaring_index::applicable_predicate;
    if let Some((col_name, _)) = applicable_predicate(predicate) {
        if let Ok(idx) = schema.index_of(col_name) {
            let field = schema.field(idx);
            if matches!(
                field.data_type(),
                arrow::datatypes::DataType::Utf8 | arrow::datatypes::DataType::LargeUtf8
            ) {
                return Some(idx);
            }
        }
    }
    None
}

fn extract_combined_filter(ops: &[ViewOp]) -> Option<Predicate> {
    let filter_preds: Vec<Predicate> = ops
        .iter()
        .filter_map(|op| {
            if let ViewOp::Filter { predicate } = op {
                Some(predicate.clone())
            } else {
                None
            }
        })
        .collect();

    match filter_preds.len() {
        0 => None,
        1 => Some(filter_preds.into_iter().next().unwrap()),
        _ => Some(Predicate::And {
            exprs: filter_preds,
        }),
    }
}

fn literal_to_f64(lit: &Literal) -> Option<f64> {
    match lit {
        Literal::Int(n) => Some(*n as f64),
        Literal::Float(f) => Some(*f),
        _ => None,
    }
}

fn mark_qualifying_rgs(
    pred: &Predicate,
    schema: &SchemaRef,
    path: &str,
    mark_cache: &MarkCache,
) -> Option<Vec<usize>> {
    match pred {
        Predicate::Eq { column, value } => {
            let col_idx = schema.index_of(column).ok()?;
            let val = literal_to_f64(value)?;
            let cache = mark_cache.read().unwrap();
            let mi = cache.get(&(path.to_string(), col_idx))?;
            Some(mi.lookup_eq(val))
        }
        Predicate::Gt { column, value } => {
            let col_idx = schema.index_of(column).ok()?;
            let val = literal_to_f64(value)?;
            let cache = mark_cache.read().unwrap();
            let mi = cache.get(&(path.to_string(), col_idx))?;
            Some(mi.lookup_gt(val))
        }
        Predicate::Gte { column, value } => {
            let col_idx = schema.index_of(column).ok()?;
            let val = literal_to_f64(value)?;
            let cache = mark_cache.read().unwrap();
            let mi = cache.get(&(path.to_string(), col_idx))?;
            Some(mi.lookup_gte(val))
        }
        Predicate::Lt { column, value } => {
            let col_idx = schema.index_of(column).ok()?;
            let val = literal_to_f64(value)?;
            let cache = mark_cache.read().unwrap();
            let mi = cache.get(&(path.to_string(), col_idx))?;
            Some(mi.lookup_lt(val))
        }
        Predicate::Lte { column, value } => {
            let col_idx = schema.index_of(column).ok()?;
            let val = literal_to_f64(value)?;
            let cache = mark_cache.read().unwrap();
            let mi = cache.get(&(path.to_string(), col_idx))?;
            Some(mi.lookup_lte(val))
        }
        Predicate::Between { column, lo, hi } => {
            let col_idx = schema.index_of(column).ok()?;
            let lo_val = literal_to_f64(lo)?;
            let hi_val = literal_to_f64(hi)?;
            let cache = mark_cache.read().unwrap();
            let mi = cache.get(&(path.to_string(), col_idx))?;
            Some(mi.lookup_between(lo_val, hi_val))
        }
        Predicate::And { exprs } => exprs
            .iter()
            .find_map(|e| mark_qualifying_rgs(e, schema, path, mark_cache)),
        _ => None,
    }
}

fn build_quick_stats(
    path: &str,
    n_rows: u64,
    n_cols: usize,
    format: &SourceFormat,
    kind: &SourceKind,
) -> Vec<QuickColumnStats> {
    if !matches!(format, SourceFormat::Parquet) || is_cloud_kind(kind) {
        return vec![];
    }
    match reader::parquet_all_quick_stats(path) {
        Ok(raw) => raw
            .into_iter()
            .take(n_cols)
            .enumerate()
            .map(|(i, (min_f, max_f, null_count))| QuickColumnStats {
                index: i,
                null_count,
                null_rate: if n_rows > 0 {
                    null_count as f64 / n_rows as f64
                } else {
                    0.0
                },
                min: min_f
                    .and_then(|f| serde_json::Number::from_f64(f).map(serde_json::Value::Number)),
                max: max_f
                    .and_then(|f| serde_json::Number::from_f64(f).map(serde_json::Value::Number)),
            })
            .collect(),
        Err(_) => vec![],
    }
}

fn is_cloud_kind(kind: &SourceKind) -> bool {
    matches!(
        kind,
        SourceKind::S3 | SourceKind::Gcs | SourceKind::AzureBlob | SourceKind::Http
    )
}

fn slice_batches(batches: &[RecordBatch], offset: usize, limit: usize) -> Vec<RecordBatch> {
    let mut result = Vec::new();
    let mut skipped = 0usize;
    let mut collected = 0usize;

    for batch in batches {
        if collected >= limit {
            break;
        }
        let n = batch.num_rows();
        if skipped + n <= offset {
            skipped += n;
            continue;
        }
        let start = offset.saturating_sub(skipped);
        let available = n - start;
        let take = available.min(limit - collected);
        result.push(batch.slice(start, take));
        skipped += n;
        collected += take;
    }
    result
}

fn project_batch_by_names(
    batch: &RecordBatch,
    names: &[String],
) -> Result<RecordBatch, arrow::error::ArrowError> {
    let indices: Vec<usize> = names
        .iter()
        .filter_map(|name| batch.schema().index_of(name).ok())
        .collect();
    batch.project(&indices)
}

fn write_jsonl(batches: &[RecordBatch]) -> Vec<u8> {
    use arrow::array::Array;
    use arrow::datatypes::DataType as Dt;
    let mut buf = Vec::new();
    for batch in batches {
        let schema = batch.schema();
        for row_idx in 0..batch.num_rows() {
            buf.push(b'{');
            let mut first = true;
            for (col_idx, field) in schema.fields().iter().enumerate() {
                if !first {
                    buf.push(b',');
                }
                first = false;
                json_write_str(&mut buf, field.name());
                buf.push(b':');
                let col = batch.column(col_idx);
                if col.is_null(row_idx) {
                    buf.extend_from_slice(b"null");
                    continue;
                }
                match col.data_type() {
                    Dt::Boolean => {
                        let v = col
                            .as_any()
                            .downcast_ref::<arrow::array::BooleanArray>()
                            .map(|a| a.value(row_idx));
                        buf.extend_from_slice(if v.unwrap_or(false) {
                            b"true"
                        } else {
                            b"false"
                        });
                    }
                    Dt::Int8
                    | Dt::Int16
                    | Dt::Int32
                    | Dt::Int64
                    | Dt::UInt8
                    | Dt::UInt16
                    | Dt::UInt32
                    | Dt::UInt64 => {
                        if let Ok(c) = arrow::compute::cast(col.as_ref(), &Dt::Int64) {
                            if let Some(a) = c.as_any().downcast_ref::<arrow::array::Int64Array>() {
                                buf.extend_from_slice(a.value(row_idx).to_string().as_bytes());
                                continue;
                            }
                        }
                        buf.extend_from_slice(b"null");
                    }
                    Dt::Float16 | Dt::Float32 | Dt::Float64 => {
                        if let Ok(c) = arrow::compute::cast(col.as_ref(), &Dt::Float64) {
                            if let Some(a) = c.as_any().downcast_ref::<arrow::array::Float64Array>()
                            {
                                let v = a.value(row_idx);
                                if v.is_finite() {
                                    buf.extend_from_slice(v.to_string().as_bytes());
                                } else {
                                    buf.extend_from_slice(b"null");
                                }
                                continue;
                            }
                        }
                        buf.extend_from_slice(b"null");
                    }
                    _ => {
                        if let Ok(c) = arrow::compute::cast(col.as_ref(), &Dt::Utf8) {
                            if let Some(a) = c.as_any().downcast_ref::<arrow::array::StringArray>()
                            {
                                if !a.is_null(row_idx) {
                                    json_write_str(&mut buf, a.value(row_idx));
                                    continue;
                                }
                            }
                        }
                        buf.extend_from_slice(b"null");
                    }
                }
            }
            buf.extend_from_slice(b"}\n");
        }
    }
    buf
}

fn json_write_str(buf: &mut Vec<u8>, s: &str) {
    buf.push(b'"');
    for c in s.chars() {
        match c {
            '"' => buf.extend_from_slice(b"\\\""),
            '\\' => buf.extend_from_slice(b"\\\\"),
            '\n' => buf.extend_from_slice(b"\\n"),
            '\r' => buf.extend_from_slice(b"\\r"),
            '\t' => buf.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                let s = format!("\\u{:04x}", c as u32);
                buf.extend_from_slice(s.as_bytes());
            }
            c => {
                let mut tmp = [0u8; 4];
                buf.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
            }
        }
    }
    buf.push(b'"');
}

fn format_data_type(dt: &arrow::datatypes::DataType) -> String {
    use arrow::datatypes::DataType;
    match dt {
        DataType::Boolean => "Boolean".to_string(),
        DataType::Int8 => "Int8".to_string(),
        DataType::Int16 => "Int16".to_string(),
        DataType::Int32 => "Int32".to_string(),
        DataType::Int64 => "Int64".to_string(),
        DataType::UInt8 => "UInt8".to_string(),
        DataType::UInt16 => "UInt16".to_string(),
        DataType::UInt32 => "UInt32".to_string(),
        DataType::UInt64 => "UInt64".to_string(),
        DataType::Float16 => "Float16".to_string(),
        DataType::Float32 => "Float32".to_string(),
        DataType::Float64 => "Float64".to_string(),
        DataType::Utf8 => "Utf8".to_string(),
        DataType::LargeUtf8 => "LargeUtf8".to_string(),
        DataType::Date32 => "Date32".to_string(),
        DataType::Date64 => "Date64".to_string(),
        DataType::Timestamp(unit, _) => format!("Timestamp({unit:?})"),
        DataType::Binary => "Binary".to_string(),
        DataType::LargeBinary => "LargeBinary".to_string(),
        DataType::List(_) => "List".to_string(),
        DataType::Struct(_) => "Struct".to_string(),
        other => format!("{other}"),
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadFormat {
    Parquet,
    Csv,
    Arrow,
    Jsonl,
}

impl DownloadFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            DownloadFormat::Parquet => "parquet",
            DownloadFormat::Csv => "csv",
            DownloadFormat::Arrow => "arrow",
            DownloadFormat::Jsonl => "jsonl",
        }
    }

    pub fn content_type(&self) -> &'static str {
        match self {
            DownloadFormat::Parquet => "application/octet-stream",
            DownloadFormat::Csv => "text/csv",
            DownloadFormat::Arrow => "application/vnd.apache.arrow.stream",
            DownloadFormat::Jsonl => "application/jsonlines",
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodegenTarget {
    DuckdbSql,
    AnsiSql,
    PythonPandas,
    PythonPolars,
    PythonDuckdb,
    Shell,
    ShellCsv,
    Dbt,
}

fn deserialize_ipc_to_batches(ipc: &[u8]) -> Result<Vec<RecordBatch>, EngineError> {
    let reader = arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(ipc), None)
        .map_err(EngineError::Arrow)?;
    reader
        .collect::<Result<Vec<_>, _>>()
        .map_err(EngineError::Arrow)
}

fn aggregate_tile_batch(batches: &[RecordBatch]) -> Result<RecordBatch, EngineError> {
    use arrow::array::{Float32Array, Float64Array, UInt8Array};
    use arrow::datatypes::{DataType, Field, Schema};

    if batches.is_empty() {
        return Ok(RecordBatch::new_empty(Arc::new(Schema::empty())));
    }

    let schema = batches[0].schema();
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    if total_rows == 0 {
        return Ok(RecordBatch::new_empty(Arc::new(Schema::empty())));
    }

    let mut fields = Vec::new();
    let mut cols: Vec<Arc<dyn arrow::array::Array>> = Vec::new();

    for field in schema.fields() {
        let arrays: Vec<&dyn arrow::array::Array> = batches
            .iter()
            .map(|b| {
                b.column_by_name(field.name())
                    .map(|c| c.as_ref())
                    .unwrap_or_else(|| b.column(0).as_ref())
            })
            .collect();

        let null_count: usize = arrays.iter().map(|a| a.null_count()).sum();
        let null_pct = null_count as f32 / total_rows as f32;

        let type_char: u8 = match field.data_type() {
            DataType::Boolean => 2,
            DataType::Utf8 | DataType::LargeUtf8 => 1,
            DataType::Null => 3,
            DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float32
            | DataType::Float64 => 0,
            _ => 1,
        };

        let mean: f64 = if type_char == 0 {
            let mut sum = 0.0f64;
            let mut count = 0u64;
            for arr in &arrays {
                if let Ok(f64_col) = arrow::compute::cast(*arr, &DataType::Float64) {
                    if let Some(fa) = f64_col.as_any().downcast_ref::<Float64Array>() {
                        for i in 0..fa.len() {
                            if !fa.is_null(i) {
                                sum += fa.value(i);
                                count += 1;
                            }
                        }
                    }
                }
            }
            if count > 0 {
                sum / count as f64
            } else {
                0.0
            }
        } else {
            0.0
        };

        fields.push(Field::new(
            format!("{}_mean", field.name()),
            DataType::Float64,
            false,
        ));
        fields.push(Field::new(
            format!("{}_null_pct", field.name()),
            DataType::Float32,
            false,
        ));
        fields.push(Field::new(
            format!("{}_type_char", field.name()),
            DataType::UInt8,
            false,
        ));

        cols.push(Arc::new(Float64Array::from(vec![mean])));
        cols.push(Arc::new(Float32Array::from(vec![null_pct])));
        cols.push(Arc::new(UInt8Array::from(vec![type_char])));
    }

    let agg_schema = Arc::new(Schema::new(fields));
    RecordBatch::try_new(agg_schema, cols).map_err(EngineError::Arrow)
}
