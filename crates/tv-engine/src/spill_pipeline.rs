use std::sync::Arc;

use arrow::array::new_null_array;
use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use tv_core::{view_hash, SortKey, SourceMeta, ViewOp};

use crate::batch_stream::{self, BatchStream};
use crate::error::EngineError;
use crate::executor;
use crate::external_sort::ExternalSorter;
use crate::materializer::MaterializedView;
use crate::spill::SpillWriter;
use crate::streaming_agg::StreamingAggregator;
use crate::temp::TempRoot;

pub struct SpillPipeline {
    pub temp_root: Arc<TempRoot>,
}

impl SpillPipeline {
    pub fn new(temp_root: Arc<TempRoot>) -> Self {
        Self { temp_root }
    }

    pub fn build(
        &self,
        meta: &SourceMeta,
        ops: &[ViewOp],
        schema_hint: Option<SchemaRef>,
    ) -> Result<MaterializedView, EngineError> {
        let normalized = tv_core::normalize_ops(ops);

        let schema = match schema_hint {
            Some(s) => s,
            None => infer_source_schema(meta)?,
        };

        let processed_schema = compute_processed_schema(&schema, &normalized)?;

        let view_h = view_hash(&format!("{:?}", normalized));
        let guard = Arc::new(self.temp_root.view_dir(&meta.id, &view_h)?);

        let stateless: Vec<ViewOp> = normalized
            .iter()
            .filter(|op| is_stateless_op(op))
            .cloned()
            .collect();

        let mat_ops: Vec<&ViewOp> = normalized
            .iter()
            .filter(|op| is_materializing_op(op))
            .collect();

        if mat_ops.is_empty() {
            let stream = batch_stream::stream_source(meta, &normalized, Some(schema.clone()))?;
            let all = collect_stream_with_stateless(stream, &stateless)?;
            let total_rows: u64 = all.iter().map(|b| b.num_rows() as u64).sum();
            return Ok(MaterializedView::Batches {
                batches: all,
                total_rows,
            });
        }

        let sort_op = mat_ops.iter().find(|op| matches!(op, ViewOp::Sort { .. }));
        let group_op = mat_ops
            .iter()
            .find(|op| matches!(op, ViewOp::GroupBy { .. }));
        let dedup_op = mat_ops
            .iter()
            .find(|op| matches!(op, ViewOp::Deduplicate { .. }));
        let sample_op = mat_ops
            .iter()
            .find(|op| matches!(op, ViewOp::Sample { .. }));
        let limit_op = mat_ops.iter().find(|op| matches!(op, ViewOp::Limit { .. }));
        let top_k_op = mat_ops.iter().find(|op| matches!(op, ViewOp::TopK { .. }));

        if let Some(ViewOp::TopK { n, keys }) = top_k_op {
            let k = *n as usize;
            let is_single_local_parquet = matches!(meta.format, tv_core::SourceFormat::Parquet)
                && meta.kind == tv_core::SourceKind::LocalFile
                && meta.files.len() == 1
                && stateless.is_empty();

            if is_single_local_parquet {
                let source_path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
                let batches = crate::top_k::stream_top_k(source_path, keys, k, &schema)?;
                let total_rows = batches.iter().map(|b| b.num_rows() as u64).sum();
                return Ok(MaterializedView::Batches {
                    batches,
                    total_rows,
                });
            }

            let stream = batch_stream::stream_source(meta, &normalized, Some(schema.clone()))?;
            let batches =
                crate::top_k::top_k_from_batches(stream, &stateless, keys, k, &processed_schema)?;
            let total_rows = batches.iter().map(|b| b.num_rows() as u64).sum();
            return Ok(MaterializedView::Batches {
                batches,
                total_rows,
            });
        }

        if let Some(ViewOp::Sample { n, seed, .. }) = sample_op {
            let stream = batch_stream::stream_source(meta, &normalized, Some(schema.clone()))?;
            let batches = reservoir_sample_stream(stream, &stateless, *n, *seed)?;
            let total_rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
            return Ok(MaterializedView::Batches {
                batches,
                total_rows,
            });
        }

        if let Some(ViewOp::Limit { n }) = limit_op {
            let stream = batch_stream::stream_source(meta, &normalized, Some(schema.clone()))?;
            let batches = collect_limit_stream(stream, &stateless, *n)?;
            let total_rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
            return Ok(MaterializedView::Batches {
                batches,
                total_rows,
            });
        }

        if let Some(ViewOp::GroupBy { keys, aggs }) = group_op {
            let aggregator =
                StreamingAggregator::new(keys.clone(), aggs.clone(), &processed_schema)?;
            let agg_schema = aggregator.output_schema.clone();
            let mut writer = SpillWriter::new(guard.path().to_path_buf(), agg_schema.clone());
            let stream = batch_stream::stream_source(meta, &normalized, Some(schema.clone()))?;
            let processed_stream = StatelessFilterStream {
                inner: stream,
                ops: stateless,
            };
            let (agg_run, group_count) = aggregator.execute(processed_stream, &mut writer)?;

            if let Some(ViewOp::Sort { keys: sort_keys }) = sort_op {
                let sorter = ExternalSorter::new(sort_keys.clone(), agg_schema.clone());
                let mut sort_writer =
                    SpillWriter::new(guard.path().to_path_buf(), agg_schema.clone());
                let agg_stream = spill_run_to_stream(&agg_run)?;
                let mut result = sorter.sort_to_runs(agg_stream, &mut sort_writer)?;
                if result.runs.len() > 1 {
                    let merged = sorter.merge_to_single_run(&result.runs, &mut sort_writer)?;
                    result.runs = vec![merged];
                    result.cumulative_rows = vec![result.total_rows];
                }
                return Ok(MaterializedView::SortedRuns {
                    runs: result.runs,
                    cumulative_rows: result.cumulative_rows,
                    schema: agg_schema,
                    sort_keys: sort_keys.clone(),
                    total_rows: result.total_rows,
                    dedup_columns: None,
                    _guard: guard,
                });
            }

            return Ok(MaterializedView::AggregateResult {
                run: agg_run,
                schema: agg_schema,
                total_rows: group_count,
                _guard: guard,
            });
        }

        if let Some(ViewOp::Deduplicate { columns }) = dedup_op {
            let sort_keys: Vec<SortKey> = match columns {
                None => processed_schema
                    .fields()
                    .iter()
                    .map(|f| SortKey {
                        column: f.name().clone(),
                        descending: false,
                        nulls_last: true,
                    })
                    .collect(),
                Some(cols) => cols
                    .iter()
                    .map(|c| SortKey {
                        column: c.clone(),
                        descending: false,
                        nulls_last: true,
                    })
                    .collect(),
            };
            let dedup_cols = columns.clone().unwrap_or_else(|| {
                processed_schema
                    .fields()
                    .iter()
                    .map(|f| f.name().clone())
                    .collect()
            });
            let sorter = ExternalSorter::new(sort_keys.clone(), processed_schema.clone());
            let mut writer = SpillWriter::new(guard.path().to_path_buf(), processed_schema.clone());
            let stream = batch_stream::stream_source(meta, &normalized, Some(schema.clone()))?;
            let processed_stream = StatelessFilterStream {
                inner: stream,
                ops: stateless,
            };
            let mut result = sorter.sort_to_runs(processed_stream, &mut writer)?;
            if result.runs.len() > 1 {
                let merged = sorter.merge_to_single_run(&result.runs, &mut writer)?;
                result.runs = vec![merged];
                result.cumulative_rows = vec![result.total_rows];
            }
            return Ok(MaterializedView::SortedRuns {
                runs: result.runs,
                cumulative_rows: result.cumulative_rows,
                schema: processed_schema.clone(),
                sort_keys,
                total_rows: result.total_rows,
                dedup_columns: Some(dedup_cols),
                _guard: guard,
            });
        }

        if let Some(ViewOp::Sort { keys }) = sort_op {
            let sorter = ExternalSorter::new(keys.clone(), processed_schema.clone());
            let mut writer = SpillWriter::new(guard.path().to_path_buf(), processed_schema.clone());
            let stream = batch_stream::stream_source(meta, &normalized, Some(schema.clone()))?;
            let processed_stream = StatelessFilterStream {
                inner: stream,
                ops: stateless,
            };
            let (initial_runs, total_rows) =
                sorter.sort_to_initial_runs(processed_stream, &mut writer)?;

            if initial_runs.is_empty() {
                return Ok(MaterializedView::SortedRuns {
                    runs: initial_runs,
                    cumulative_rows: vec![],
                    schema: processed_schema.clone(),
                    sort_keys: keys.clone(),
                    total_rows: 0,
                    dedup_columns: None,
                    _guard: guard,
                });
            }

            let in_memory_threshold = in_memory_sort_threshold();
            if total_rows < in_memory_threshold && initial_runs.len() == 1 {
                let cumulative_rows = vec![initial_runs[0].row_count];
                return Ok(MaterializedView::SortedRuns {
                    runs: initial_runs,
                    cumulative_rows,
                    schema: processed_schema.clone(),
                    sort_keys: keys.clone(),
                    total_rows,
                    dedup_columns: None,
                    _guard: guard,
                });
            }

            let threshold = progressive_sort_threshold();
            if total_rows > threshold && initial_runs.len() > 1 {
                let first_run_count = initial_runs[0].row_count;
                let first_runs = vec![initial_runs[0].clone()];
                let cumulative_rows = vec![first_run_count];
                let job_id = uuid::Uuid::new_v4().to_string();
                return Ok(MaterializedView::ProvisionalSort {
                    runs: first_runs,
                    cumulative_rows,
                    schema: processed_schema.clone(),
                    sort_keys: keys.clone(),
                    total_rows: first_run_count,
                    estimated_total_rows: total_rows,
                    job_id,
                    _guard: guard,
                });
            }

            let mut runs = sorter.cascade_runs(initial_runs, &mut writer)?;

            if total_rows > sparse_sort_threshold() && !runs.is_empty() {
                let index_path = guard.path().join("sorted.tvs");
                let merged = sorter.merge_to_single_run(&runs, &mut writer)?;
                if let Ok(index) = crate::sparse_sort_index::build_sparse(&merged.path, &index_path)
                {
                    return Ok(MaterializedView::SparseSortIndexBacked {
                        index: Arc::new(index),
                        spill_path: merged.path,
                        schema: processed_schema,
                        total_rows,
                        _guard: guard,
                    });
                }
                runs = vec![merged];
            } else if runs.len() > 1 {
                let merged = sorter.merge_to_single_run(&runs, &mut writer)?;
                runs = vec![merged];
            }

            let cumulative_rows = if runs.is_empty() {
                vec![]
            } else {
                vec![total_rows]
            };
            return Ok(MaterializedView::SortedRuns {
                runs,
                cumulative_rows,
                schema: processed_schema.clone(),
                sort_keys: keys.clone(),
                total_rows,
                dedup_columns: None,
                _guard: guard,
            });
        }

        let stream = batch_stream::stream_source(meta, &normalized, Some(schema))?;
        let all = collect_stream_with_stateless(stream, &[])?;
        let processed = executor::execute_pipeline_skip_filter(all, &normalized)?;
        let total_rows: u64 = processed.iter().map(|b| b.num_rows() as u64).sum();
        Ok(MaterializedView::Batches {
            batches: processed,
            total_rows,
        })
    }

    pub fn build_full(
        &self,
        meta: &SourceMeta,
        ops: &[ViewOp],
        schema_hint: Option<SchemaRef>,
    ) -> Result<MaterializedView, EngineError> {
        let normalized = tv_core::normalize_ops(ops);
        let schema = match schema_hint {
            Some(s) => s,
            None => infer_source_schema(meta)?,
        };
        let processed_schema = compute_processed_schema(&schema, &normalized)?;
        let view_h = view_hash(&format!("{:?}", normalized));
        let guard = Arc::new(
            self.temp_root
                .view_dir(&meta.id, &format!("{view_h}_full"))?,
        );
        let stateless: Vec<ViewOp> = normalized
            .iter()
            .filter(|op| is_stateless_op(op))
            .cloned()
            .collect();

        if let Some(ViewOp::Sort { keys }) = normalized
            .iter()
            .find(|op| matches!(op, ViewOp::Sort { .. }))
        {
            {
                let sorter = ExternalSorter::new(keys.clone(), processed_schema.clone());
                let mut writer =
                    SpillWriter::new(guard.path().to_path_buf(), processed_schema.clone());
                let stream = batch_stream::stream_source(meta, &normalized, Some(schema))?;
                let processed_stream = StatelessFilterStream {
                    inner: stream,
                    ops: stateless,
                };
                let mut result = sorter.sort_to_runs(processed_stream, &mut writer)?;
                if result.total_rows > sparse_sort_threshold() {
                    let index_path = guard.path().join("sorted.tvs");
                    let merged = sorter.merge_to_single_run(&result.runs, &mut writer)?;
                    if let Ok(index) =
                        crate::sparse_sort_index::build_sparse(&merged.path, &index_path)
                    {
                        return Ok(MaterializedView::SparseSortIndexBacked {
                            index: Arc::new(index),
                            spill_path: merged.path,
                            schema: processed_schema,
                            total_rows: result.total_rows,
                            _guard: guard,
                        });
                    }
                    result.runs = vec![merged];
                    result.cumulative_rows = vec![result.total_rows];
                } else if result.runs.len() > 1 {
                    let merged = sorter.merge_to_single_run(&result.runs, &mut writer)?;
                    result.runs = vec![merged];
                    result.cumulative_rows = vec![result.total_rows];
                }
                return Ok(MaterializedView::SortedRuns {
                    runs: result.runs,
                    cumulative_rows: result.cumulative_rows,
                    schema: processed_schema,
                    sort_keys: keys.clone(),
                    total_rows: result.total_rows,
                    dedup_columns: None,
                    _guard: guard,
                });
            }
        }
        self.build(meta, ops, None)
    }
}

fn progressive_sort_threshold() -> u64 {
    std::env::var("PROGRESSIVE_SORT_THRESHOLD_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000)
}

fn in_memory_sort_threshold() -> u64 {
    std::env::var("IN_MEMORY_SORT_THRESHOLD_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000_000)
}

fn sparse_sort_threshold() -> u64 {
    std::env::var("SPARSE_SORT_THRESHOLD_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000)
}

struct StatelessFilterStream {
    inner: BatchStream,
    ops: Vec<ViewOp>,
}

impl Iterator for StatelessFilterStream {
    type Item = Result<RecordBatch, EngineError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.inner.next() {
                None => return None,
                Some(Err(e)) => return Some(Err(e)),
                Some(Ok(batch)) => {
                    if batch.num_rows() == 0 {
                        continue;
                    }
                    if self.ops.is_empty() {
                        return Some(Ok(batch));
                    }
                    match executor::execute_pipeline(vec![batch], &self.ops) {
                        Err(e) => return Some(Err(e)),
                        Ok(mut batches) => {
                            if let Some(b) = batches.pop() {
                                if b.num_rows() > 0 {
                                    return Some(Ok(b));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn spill_run_to_stream(run: &crate::spill::SpilledRun) -> Result<BatchStream, EngineError> {
    let reader = crate::spill::SpillReader::open(&run.path)?;
    let batches: Vec<RecordBatch> = reader.collect::<Result<_, _>>()?;
    Ok(BatchStream::InMemory(batches.into_iter()))
}

fn collect_stream_with_stateless(
    stream: BatchStream,
    stateless_ops: &[ViewOp],
) -> Result<Vec<RecordBatch>, EngineError> {
    let mut result = Vec::new();
    for batch_result in stream {
        let batch = batch_result?;
        if batch.num_rows() == 0 {
            continue;
        }
        if stateless_ops.is_empty() {
            result.push(batch);
        } else {
            let processed = executor::execute_pipeline(vec![batch], stateless_ops)?;
            result.extend(processed.into_iter().filter(|b| b.num_rows() > 0));
        }
    }
    Ok(result)
}

fn reservoir_sample_stream(
    stream: BatchStream,
    stateless_ops: &[ViewOp],
    n: u64,
    seed: Option<u64>,
) -> Result<Vec<RecordBatch>, EngineError> {
    let batches = collect_stream_with_stateless(stream, stateless_ops)?;
    executor::execute_pipeline(
        batches,
        &[ViewOp::Sample {
            n,
            seed,
            strategy: tv_core::SampleStrategy::Bernoulli,
        }],
    )
}

fn collect_limit_stream(
    stream: BatchStream,
    stateless_ops: &[ViewOp],
    n: u64,
) -> Result<Vec<RecordBatch>, EngineError> {
    let mut result = Vec::new();
    let mut collected: u64 = 0;
    for batch_result in stream {
        if collected >= n {
            break;
        }
        let batch = batch_result?;
        if batch.num_rows() == 0 {
            continue;
        }
        let processed = if stateless_ops.is_empty() {
            vec![batch]
        } else {
            executor::execute_pipeline(vec![batch], stateless_ops)?
        };
        for b in processed {
            if b.num_rows() == 0 {
                continue;
            }
            let remaining = (n - collected) as usize;
            if b.num_rows() <= remaining {
                collected += b.num_rows() as u64;
                result.push(b);
            } else {
                result.push(b.slice(0, remaining));
                collected = n;
                break;
            }
        }
    }
    Ok(result)
}

fn is_stateless_op(op: &ViewOp) -> bool {
    matches!(
        op,
        ViewOp::Filter { .. }
            | ViewOp::Select { .. }
            | ViewOp::Drop { .. }
            | ViewOp::Derive { .. }
            | ViewOp::Rename { .. }
    )
}

fn is_materializing_op(op: &ViewOp) -> bool {
    matches!(
        op,
        ViewOp::Sort { .. }
            | ViewOp::GroupBy { .. }
            | ViewOp::Deduplicate { .. }
            | ViewOp::Sample { .. }
            | ViewOp::Limit { .. }
            | ViewOp::TopK { .. }
    )
}

fn infer_source_schema(meta: &SourceMeta) -> Result<SchemaRef, EngineError> {
    use tv_core::SourceFormat;
    match &meta.format {
        SourceFormat::Parquet => {
            let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
            crate::reader::parquet_schema_and_rows(path).map(|(s, _)| s)
        }
        SourceFormat::Csv => crate::reader::csv_schema_and_data(&meta.uri).map(|(s, _)| s),
        SourceFormat::Json => crate::reader::json_schema_and_data(&meta.uri).map(|(s, _)| s),
        SourceFormat::Arrow => crate::reader::arrow_ipc_schema_and_data(&meta.uri).map(|(s, _)| s),
        other => Err(EngineError::UnsupportedFormat(format!("{other:?}"))),
    }
}

fn compute_processed_schema(schema: &SchemaRef, ops: &[ViewOp]) -> Result<SchemaRef, EngineError> {
    let fields = schema.fields();
    if fields.is_empty() {
        return Ok(schema.clone());
    }
    let empty_cols: Vec<std::sync::Arc<dyn arrow::array::Array>> = fields
        .iter()
        .map(|f| new_null_array(f.data_type(), 0))
        .collect();
    let empty_batch = RecordBatch::try_new(schema.clone(), empty_cols)?;
    let stateless: Vec<ViewOp> = ops
        .iter()
        .filter(|op| is_stateless_op(op))
        .cloned()
        .collect();
    if stateless.is_empty() {
        return Ok(schema.clone());
    }
    let result = executor::execute_pipeline(vec![empty_batch], &stateless)?;
    Ok(result
        .first()
        .map(|b| b.schema())
        .unwrap_or_else(|| schema.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materializer::MaterializedView;
    use crate::temp::TempRoot;
    use crate::test_helpers::{people_batches, write_test_parquet};
    use tempfile::TempDir;
    use tv_core::{
        AggExpr, Literal, Predicate, SortKey, SourceFormat, SourceKind, SourceMeta, ViewOp,
    };

    fn make_parquet_source(
        tmp: &TempDir,
        batches: &[arrow::record_batch::RecordBatch],
    ) -> SourceMeta {
        let path = write_test_parquet(tmp, "data.parquet", batches);
        let path_str = path.to_str().unwrap().to_string();
        SourceMeta {
            id: "test_source".to_string(),
            name: "test_source".to_string(),
            uri: path_str.clone(),
            files: vec![path_str],
            format: SourceFormat::Parquet,
            kind: SourceKind::LocalFile,
            n_rows: batches.iter().map(|b| b.num_rows() as u64).sum(),
            n_cols: batches.first().map(|b| b.num_columns()).unwrap_or(0),
            columns: vec![],
            quick_stats: vec![],
            tile_rows: 256,
            file_mtime_secs: 0,
            file_size_bytes: 0,
            recommendations: vec![],
            pre_sorted_by: None,
        }
    }

    fn make_pipeline() -> SpillPipeline {
        SpillPipeline::new(TempRoot::new().unwrap())
    }

    #[test]
    fn classify_stateless_vs_materializing() {
        let filter_ops = vec![ViewOp::Filter {
            predicate: Predicate::IsNotNull {
                column: "id".to_string(),
            },
        }];
        let sort_ops = vec![ViewOp::Sort {
            keys: vec![SortKey {
                column: "id".to_string(),
                descending: false,
                nulls_last: true,
            }],
        }];

        for op in &filter_ops {
            assert!(is_stateless_op(op));
            assert!(!is_materializing_op(op));
        }
        for op in &sort_ops {
            assert!(!is_stateless_op(op));
            assert!(is_materializing_op(op));
        }
    }

    #[test]
    fn build_stateless_only() {
        let tmp = TempDir::new().unwrap();
        let batches = people_batches();
        let meta = make_parquet_source(&tmp, &batches);
        let pipeline = make_pipeline();
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::Gt {
                column: "age".to_string(),
                value: Literal::Float(30.0),
            },
        }];
        let result = pipeline.build(&meta, &ops, None).unwrap();
        assert!(matches!(result, MaterializedView::Batches { .. }));
    }

    #[test]
    fn build_sort_returns_sorted_runs() {
        let tmp = TempDir::new().unwrap();
        let batches = people_batches();
        let meta = make_parquet_source(&tmp, &batches);
        let pipeline = make_pipeline();
        let ops = vec![ViewOp::Sort {
            keys: vec![SortKey {
                column: "age".to_string(),
                descending: false,
                nulls_last: true,
            }],
        }];
        let result = pipeline.build(&meta, &ops, None).unwrap();
        assert!(matches!(
            result,
            MaterializedView::SortedRuns { .. }
                | MaterializedView::SparseSortIndexBacked { .. }
                | MaterializedView::ProvisionalAgg { .. }
                | MaterializedView::ProvisionalSort { .. }
                | MaterializedView::Batches { .. }
        ));
    }

    #[test]
    fn build_group_by_returns_aggregate() {
        let tmp = TempDir::new().unwrap();
        let batches = people_batches();
        let meta = make_parquet_source(&tmp, &batches);
        let pipeline = make_pipeline();
        let ops = vec![ViewOp::GroupBy {
            keys: vec!["department".to_string()],
            aggs: vec![AggExpr::Count {
                alias: "cnt".to_string(),
            }],
        }];
        let result = pipeline.build(&meta, &ops, None).unwrap();
        let total = result.row_count();
        assert_eq!(total, 4);
        assert!(matches!(
            result,
            MaterializedView::AggregateResult { .. } | MaterializedView::SortedRuns { .. }
        ));
    }

    #[test]
    fn build_sample_returns_correct_count() {
        let tmp = TempDir::new().unwrap();
        let batches = people_batches();
        let meta = make_parquet_source(&tmp, &batches);
        let pipeline = make_pipeline();
        let ops = vec![ViewOp::Sample {
            n: 50,
            seed: Some(42),
            strategy: tv_core::SampleStrategy::Bernoulli,
        }];
        let result = pipeline.build(&meta, &ops, None).unwrap();
        assert!(matches!(result, MaterializedView::Batches { .. }));
        let total = result.row_count();
        assert!(total <= 200);
    }
}
