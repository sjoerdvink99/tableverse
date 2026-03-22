use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use object_store::aws::AmazonS3Builder;
use object_store::azure::MicrosoftAzureBuilder;
use object_store::gcp::GoogleCloudStorageBuilder;
use object_store::http::HttpBuilder;
use object_store::{path::Path as ObjPath, ObjectStore};
use parquet::arrow::arrow_reader::{
    ArrowPredicateFn, ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
    RowFilter, RowSelection, RowSelector,
};
use parquet::arrow::async_reader::ParquetObjectReader;
use parquet::arrow::ProjectionMask;
use parquet::file::metadata::{ParquetMetaData, RowGroupMetaData};
use parquet::file::statistics::Statistics;
use tv_core::{Literal, Predicate, SourceFormat};
use url::Url;

use crate::error::EngineError;
use crate::executor::predicate_to_bool_array;

type ColMinMaxNull = (Option<f64>, Option<f64>, u64);
type RgColStats = (Option<f64>, Option<f64>, Option<u64>, u64);

#[derive(Clone, Debug)]
enum RgKeyBound {
    Float(f64),
    Text(String),
}

fn statistics_to_key_bounds(stats: &Statistics) -> Option<(RgKeyBound, RgKeyBound)> {
    match stats {
        Statistics::Int32(v) => Some((
            RgKeyBound::Float((*v.min_opt()?) as f64),
            RgKeyBound::Float((*v.max_opt()?) as f64),
        )),
        Statistics::Int64(v) => Some((
            RgKeyBound::Float((*v.min_opt()?) as f64),
            RgKeyBound::Float((*v.max_opt()?) as f64),
        )),
        Statistics::Float(v) => Some((
            RgKeyBound::Float((*v.min_opt()?) as f64),
            RgKeyBound::Float((*v.max_opt()?) as f64),
        )),
        Statistics::Double(v) => Some((
            RgKeyBound::Float(*v.min_opt()?),
            RgKeyBound::Float(*v.max_opt()?),
        )),
        Statistics::ByteArray(v) => {
            let min_bytes = v.min_opt()?;
            let max_bytes = v.max_opt()?;
            let min_str = std::str::from_utf8(min_bytes.data()).ok()?.to_string();
            let max_str = std::str::from_utf8(max_bytes.data()).ok()?.to_string();
            Some((RgKeyBound::Text(min_str), RgKeyBound::Text(max_str)))
        }
        _ => None,
    }
}

fn rg_col_key_bounds(
    col_name: &str,
    rg: &RowGroupMetaData,
    schema: &SchemaRef,
) -> Option<(RgKeyBound, RgKeyBound, Option<u64>, u64)> {
    let col_idx = schema.index_of(col_name).ok()?;
    if col_idx >= rg.num_columns() {
        return None;
    }
    let col_meta = rg.column(col_idx);
    let num_rows = rg.num_rows() as u64;
    let stats = col_meta.statistics()?;
    let null_count = stats.null_count_opt();
    let (min, max) = statistics_to_key_bounds(stats)?;
    Some((min, max, null_count, num_rows))
}

fn page_index_enabled() -> bool {
    std::env::var("PARQUET_PAGE_INDEX")
        .ok()
        .and_then(|s| s.parse::<bool>().ok())
        .unwrap_or(true)
}

pub fn parquet_schema_and_rows(path: &str) -> Result<(SchemaRef, u64), EngineError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let schema = builder.schema().clone();
    let n_rows = builder.metadata().file_metadata().num_rows() as u64;
    Ok((schema, n_rows))
}

pub fn parquet_schema_rows_and_metadata(
    path: &str,
) -> Result<(SchemaRef, u64, Arc<ParquetMetaData>), EngineError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new_with_options(
        file,
        ArrowReaderOptions::new().with_page_index(page_index_enabled()),
    )?;
    let schema = builder.schema().clone();
    let metadata = builder.metadata().clone();
    let n_rows = metadata.file_metadata().num_rows() as u64;
    Ok((schema, n_rows, metadata))
}

pub fn read_parquet_tile(
    path: &str,
    row_offset: usize,
    col_indices: &[usize],
    rows: usize,
    cached_metadata: Option<Arc<ParquetMetaData>>,
) -> Result<Vec<RecordBatch>, EngineError> {
    let options = ArrowReaderOptions::new().with_page_index(page_index_enabled());
    let file = File::open(path)?;
    let builder = if let Some(m) = cached_metadata {
        match ArrowReaderMetadata::try_new(m, options) {
            Ok(arm) => ParquetRecordBatchReaderBuilder::new_with_metadata(file, arm),
            Err(_) => {
                drop(file);
                let f2 = File::open(path)?;
                ParquetRecordBatchReaderBuilder::try_new_with_options(
                    f2,
                    ArrowReaderOptions::new().with_page_index(page_index_enabled()),
                )?
            }
        }
    } else {
        ParquetRecordBatchReaderBuilder::try_new_with_options(
            file,
            ArrowReaderOptions::new().with_page_index(page_index_enabled()),
        )?
    };
    if rows == 0 {
        return Ok(vec![]);
    }
    let metadata = builder.metadata().clone();
    let n_rgs = metadata.num_row_groups();
    let end_row = row_offset + rows;
    let mut rg_cursor = 0usize;
    let mut target_rgs: Vec<usize> = Vec::new();
    let mut selectors: Vec<RowSelector> = Vec::new();
    let mut rows_remaining = rows;
    for rg_idx in 0..n_rgs {
        let rg_rows = metadata.row_group(rg_idx).num_rows() as usize;
        let rg_end = rg_cursor + rg_rows;
        if rg_end > row_offset && rg_cursor < end_row {
            let skip_in_rg = if target_rgs.is_empty() {
                row_offset - rg_cursor
            } else {
                0
            };
            let available = rg_rows.saturating_sub(skip_in_rg);
            let take = available.min(rows_remaining);
            target_rgs.push(rg_idx);
            if skip_in_rg > 0 {
                selectors.push(RowSelector::skip(skip_in_rg));
            }
            selectors.push(RowSelector::select(take));
            rows_remaining -= take;
        }
        rg_cursor = rg_end;
        if rg_cursor >= end_row || rows_remaining == 0 {
            break;
        }
    }
    if target_rgs.is_empty() {
        return Ok(vec![]);
    }
    let parquet_schema = builder.parquet_schema();
    let mask = ProjectionMask::roots(parquet_schema, col_indices.iter().copied());
    let selection: RowSelection = selectors.into();
    let reader = builder
        .with_row_groups(target_rgs)
        .with_projection(mask)
        .with_row_selection(selection)
        .build()?;
    reader.collect::<Result<_, _>>().map_err(EngineError::Arrow)
}

pub fn read_parquet_full(
    path: &str,
    col_indices: Option<&[usize]>,
) -> Result<Vec<RecordBatch>, EngineError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = if let Some(cols) = col_indices {
        let parquet_schema = builder.parquet_schema();
        let mask = ProjectionMask::roots(parquet_schema, cols.iter().copied());
        builder.with_projection(mask).build()?
    } else {
        builder.build()?
    };
    reader.collect::<Result<_, _>>().map_err(EngineError::Arrow)
}

pub fn read_parquet_full_with_filter(
    path: &str,
    predicate: &Predicate,
    schema: &SchemaRef,
) -> Result<Vec<RecordBatch>, EngineError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new_with_options(
        file,
        ArrowReaderOptions::new().with_page_index(page_index_enabled()),
    )?;
    let parquet_schema = builder.parquet_schema();

    let pred_col_indices = predicate_column_indices(predicate, schema);
    let pred_mask = ProjectionMask::roots(parquet_schema, pred_col_indices.iter().copied());

    let cloned = predicate.clone();
    let row_filter = RowFilter::new(vec![Box::new(ArrowPredicateFn::new(
        pred_mask,
        move |batch| {
            predicate_to_bool_array(&batch, &cloned)
                .map_err(|e| arrow::error::ArrowError::ExternalError(Box::new(e)))
        },
    ))]);

    let reader = builder.with_row_filter(row_filter).build()?;
    reader.collect::<Result<_, _>>().map_err(EngineError::Arrow)
}

#[allow(clippy::too_many_arguments)]
pub fn read_parquet_filtered_tile(
    path: &str,
    predicate: &Predicate,
    schema: &SchemaRef,
    bloom: Option<&crate::bloom_index::BloomIndex>,
    roaring: Option<&crate::roaring_index::RoaringIndex>,
    row_offset: usize,
    rows: usize,
    cached_metadata: Option<Arc<ParquetMetaData>>,
    col_selection: Option<&[usize]>,
    mark_rgs: Option<&[usize]>,
) -> Result<Vec<RecordBatch>, EngineError> {
    let options = ArrowReaderOptions::new().with_page_index(page_index_enabled());
    let file = File::open(path)?;

    let (builder, metadata) = if let Some(m) = cached_metadata {
        match ArrowReaderMetadata::try_new(m.clone(), options) {
            Ok(arm) => {
                let b = ParquetRecordBatchReaderBuilder::new_with_metadata(file, arm);
                (b, m)
            }
            Err(_) => {
                drop(file);
                let f2 = File::open(path)?;
                let b = ParquetRecordBatchReaderBuilder::try_new_with_options(
                    f2,
                    ArrowReaderOptions::new().with_page_index(page_index_enabled()),
                )?;
                let md = b.metadata().clone();
                (b, md)
            }
        }
    } else {
        let b = ParquetRecordBatchReaderBuilder::try_new_with_options(
            file,
            ArrowReaderOptions::new().with_page_index(page_index_enabled()),
        )?;
        let md = b.metadata().clone();
        (b, md)
    };
    let n_rgs = metadata.num_row_groups();

    let qualifying_rgs: Vec<usize> = if let Some(ri) = roaring {
        if let Some((col_name, values)) = crate::roaring_index::applicable_predicate(predicate) {
            if schema.index_of(col_name).is_ok() {
                if values.len() == 1 {
                    ri.row_groups_for_eq(values[0], n_rgs)
                        .unwrap_or_else(|| (0..n_rgs).collect())
                } else {
                    ri.row_groups_for_in(&values, n_rgs)
                }
            } else {
                (0..n_rgs)
                    .filter(|&i| {
                        !can_prune_row_group(
                            predicate,
                            metadata.row_group(i),
                            schema,
                            bloom.map(|b| (b, i)),
                        )
                    })
                    .collect()
            }
        } else {
            (0..n_rgs)
                .filter(|&i| {
                    !can_prune_row_group(
                        predicate,
                        metadata.row_group(i),
                        schema,
                        bloom.map(|b| (b, i)),
                    )
                })
                .collect()
        }
    } else {
        (0..n_rgs)
            .filter(|&i| {
                !can_prune_row_group(
                    predicate,
                    metadata.row_group(i),
                    schema,
                    bloom.map(|b| (b, i)),
                )
            })
            .collect()
    };
    let qualifying_rgs: Vec<usize> = if let Some(mark) = mark_rgs {
        let mark_set: std::collections::HashSet<usize> = mark.iter().copied().collect();
        qualifying_rgs
            .into_iter()
            .filter(|i| mark_set.contains(i))
            .collect()
    } else {
        qualifying_rgs
    };

    if qualifying_rgs.is_empty() {
        return Ok(vec![]);
    }

    let pred_col_indices = predicate_column_indices(predicate, schema);

    let read_col_indices: Vec<usize> = if let Some(sel) = col_selection {
        let mut merged: Vec<usize> = pred_col_indices
            .iter()
            .copied()
            .chain(sel.iter().copied())
            .collect();
        merged.sort_unstable();
        merged.dedup();
        merged
    } else {
        pred_col_indices.clone()
    };

    let pred_mask =
        ProjectionMask::roots(builder.parquet_schema(), pred_col_indices.iter().copied());
    let read_mask =
        ProjectionMask::roots(builder.parquet_schema(), read_col_indices.iter().copied());
    let cloned = predicate.clone();
    let row_filter = RowFilter::new(vec![Box::new(ArrowPredicateFn::new(
        pred_mask,
        move |batch| {
            predicate_to_bool_array(&batch, &cloned)
                .map_err(|e| arrow::error::ArrowError::ExternalError(Box::new(e)))
        },
    ))]);

    let reader = builder
        .with_row_groups(qualifying_rgs)
        .with_projection(read_mask)
        .with_row_filter(row_filter)
        .build()?;

    let mut result = Vec::new();
    let mut skipped = 0usize;
    let mut collected = 0usize;

    for batch_result in reader {
        if collected >= rows {
            break;
        }
        let batch = batch_result.map_err(EngineError::Arrow)?;
        let n = batch.num_rows();
        if skipped + n <= row_offset {
            skipped += n;
            continue;
        }
        let start = row_offset.saturating_sub(skipped);
        let available = n - start;
        let take = available.min(rows - collected);
        let sliced = batch.slice(start, take);
        skipped += n;
        collected += take;

        if let Some(sel) = col_selection {
            let projected = project_batch_by_full_schema_indices(&sliced, &read_col_indices, sel)?;
            result.push(projected);
        } else {
            result.push(sliced);
        }
    }

    Ok(result)
}

fn project_batch_by_full_schema_indices(
    batch: &RecordBatch,
    read_col_indices: &[usize],
    selection: &[usize],
) -> Result<RecordBatch, EngineError> {
    let within_batch_cols: Vec<usize> = selection
        .iter()
        .filter_map(|full_idx| read_col_indices.binary_search(full_idx).ok())
        .collect();
    let columns: Vec<_> = within_batch_cols
        .iter()
        .map(|&i| batch.column(i).clone())
        .collect();
    let fields: Vec<_> = within_batch_cols
        .iter()
        .map(|&i| batch.schema().field(i).clone())
        .collect();
    let schema = Arc::new(arrow::datatypes::Schema::new(fields));
    RecordBatch::try_new(schema, columns).map_err(EngineError::Arrow)
}

pub fn build_filter_rg_index(
    path: &str,
    predicate: &Predicate,
    schema: &SchemaRef,
    bloom: Option<&crate::bloom_index::BloomIndex>,
    roaring: Option<&crate::roaring_index::RoaringIndex>,
    cached_metadata: Option<Arc<ParquetMetaData>>,
) -> Result<Vec<u64>, EngineError> {
    let metadata = if let Some(m) = cached_metadata {
        m
    } else {
        let file = File::open(path)?;
        let builder = ParquetRecordBatchReaderBuilder::try_new_with_options(
            file,
            ArrowReaderOptions::new().with_page_index(page_index_enabled()),
        )?;
        builder.metadata().clone()
    };
    let n_rgs = metadata.num_row_groups();

    let qualifying_rgs: Vec<usize> = if let Some(ri) = roaring {
        if let Some((col_name, values)) = crate::roaring_index::applicable_predicate(predicate) {
            if schema.index_of(col_name).is_ok() {
                if values.len() == 1 {
                    ri.row_groups_for_eq(values[0], n_rgs)
                        .unwrap_or_else(|| (0..n_rgs).collect())
                } else {
                    ri.row_groups_for_in(&values, n_rgs)
                }
            } else {
                (0..n_rgs)
                    .filter(|&i| {
                        !can_prune_row_group(
                            predicate,
                            metadata.row_group(i),
                            schema,
                            bloom.map(|b| (b, i)),
                        )
                    })
                    .collect()
            }
        } else {
            (0..n_rgs)
                .filter(|&i| {
                    !can_prune_row_group(
                        predicate,
                        metadata.row_group(i),
                        schema,
                        bloom.map(|b| (b, i)),
                    )
                })
                .collect()
        }
    } else {
        (0..n_rgs)
            .filter(|&i| {
                !can_prune_row_group(
                    predicate,
                    metadata.row_group(i),
                    schema,
                    bloom.map(|b| (b, i)),
                )
            })
            .collect()
    };

    if qualifying_rgs.is_empty() {
        return Ok(vec![]);
    }

    let pred_col_indices = predicate_column_indices(predicate, schema);
    let mut cumulative: Vec<u64> = Vec::with_capacity(qualifying_rgs.len());
    let mut running = 0u64;

    for &rg_i in &qualifying_rgs {
        let file_i = File::open(path)?;
        let builder_i = ParquetRecordBatchReaderBuilder::try_new(file_i)?;
        let pred_mask =
            ProjectionMask::roots(builder_i.parquet_schema(), pred_col_indices.iter().copied());
        let pred_mask_proj =
            ProjectionMask::roots(builder_i.parquet_schema(), pred_col_indices.iter().copied());
        let cloned = predicate.clone();
        let row_filter = RowFilter::new(vec![Box::new(ArrowPredicateFn::new(
            pred_mask,
            move |batch| {
                predicate_to_bool_array(&batch, &cloned)
                    .map_err(|e| arrow::error::ArrowError::ExternalError(Box::new(e)))
            },
        ))]);
        let reader = builder_i
            .with_row_groups(vec![rg_i])
            .with_projection(pred_mask_proj)
            .with_row_filter(row_filter)
            .build()?;
        let count: u64 = reader
            .map(|b| b.map(|b| b.num_rows() as u64).map_err(EngineError::Arrow))
            .try_fold(0u64, |acc, r| r.map(|n| acc + n))?;
        running += count;
        cumulative.push(running);
    }

    Ok(cumulative)
}

#[allow(clippy::too_many_arguments)]
pub fn read_parquet_filtered_tile_indexed(
    path: &str,
    predicate: &Predicate,
    schema: &SchemaRef,
    bloom: Option<&crate::bloom_index::BloomIndex>,
    roaring: Option<&crate::roaring_index::RoaringIndex>,
    row_offset: usize,
    rows: usize,
    rg_cumulative: &[u64],
    col_selection: Option<&[usize]>,
    mark_rgs: Option<&[usize]>,
) -> Result<Vec<RecordBatch>, EngineError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new_with_options(
        file,
        ArrowReaderOptions::new().with_page_index(page_index_enabled()),
    )?;
    let metadata = builder.metadata().clone();
    let n_rgs = metadata.num_row_groups();

    let qualifying_rgs: Vec<usize> = if let Some(ri) = roaring {
        if let Some((col_name, values)) = crate::roaring_index::applicable_predicate(predicate) {
            if schema.index_of(col_name).is_ok() {
                if values.len() == 1 {
                    ri.row_groups_for_eq(values[0], n_rgs)
                        .unwrap_or_else(|| (0..n_rgs).collect())
                } else {
                    ri.row_groups_for_in(&values, n_rgs)
                }
            } else {
                (0..n_rgs)
                    .filter(|&i| {
                        !can_prune_row_group(
                            predicate,
                            metadata.row_group(i),
                            schema,
                            bloom.map(|b| (b, i)),
                        )
                    })
                    .collect()
            }
        } else {
            (0..n_rgs)
                .filter(|&i| {
                    !can_prune_row_group(
                        predicate,
                        metadata.row_group(i),
                        schema,
                        bloom.map(|b| (b, i)),
                    )
                })
                .collect()
        }
    } else {
        (0..n_rgs)
            .filter(|&i| {
                !can_prune_row_group(
                    predicate,
                    metadata.row_group(i),
                    schema,
                    bloom.map(|b| (b, i)),
                )
            })
            .collect()
    };

    if qualifying_rgs.is_empty() || rg_cumulative.is_empty() {
        return Ok(vec![]);
    }

    let start_rg_idx = rg_cumulative.partition_point(|&c| c <= row_offset as u64);
    if start_rg_idx >= qualifying_rgs.len() {
        return Ok(vec![]);
    }

    let prior_count = if start_rg_idx > 0 {
        rg_cumulative[start_rg_idx - 1] as usize
    } else {
        0
    };
    let adjusted_offset = row_offset - prior_count;
    let relevant_rgs: Vec<usize> = if let Some(mark) = mark_rgs {
        let mark_set: std::collections::HashSet<usize> = mark.iter().copied().collect();
        qualifying_rgs[start_rg_idx..]
            .iter()
            .copied()
            .filter(|i| mark_set.contains(i))
            .collect()
    } else {
        qualifying_rgs[start_rg_idx..].to_vec()
    };

    let pred_col_indices = predicate_column_indices(predicate, schema);

    let read_col_indices: Vec<usize> = if let Some(sel) = col_selection {
        let mut merged: Vec<usize> = pred_col_indices
            .iter()
            .copied()
            .chain(sel.iter().copied())
            .collect();
        merged.sort_unstable();
        merged.dedup();
        merged
    } else {
        pred_col_indices.clone()
    };

    let pred_mask =
        ProjectionMask::roots(builder.parquet_schema(), pred_col_indices.iter().copied());
    let read_mask =
        ProjectionMask::roots(builder.parquet_schema(), read_col_indices.iter().copied());
    let cloned = predicate.clone();
    let row_filter = RowFilter::new(vec![Box::new(ArrowPredicateFn::new(
        pred_mask,
        move |batch| {
            predicate_to_bool_array(&batch, &cloned)
                .map_err(|e| arrow::error::ArrowError::ExternalError(Box::new(e)))
        },
    ))]);

    let reader = builder
        .with_row_groups(relevant_rgs)
        .with_projection(read_mask)
        .with_row_filter(row_filter)
        .build()?;

    let mut result = Vec::new();
    let mut skipped = 0usize;
    let mut collected = 0usize;

    for batch_result in reader {
        if collected >= rows {
            break;
        }
        let batch = batch_result.map_err(EngineError::Arrow)?;
        let n = batch.num_rows();
        if skipped + n <= adjusted_offset {
            skipped += n;
            continue;
        }
        let start = adjusted_offset.saturating_sub(skipped);
        let available = n - start;
        let take = available.min(rows - collected);
        let sliced = batch.slice(start, take);
        skipped += n;
        collected += take;

        if let Some(sel) = col_selection {
            let projected = project_batch_by_full_schema_indices(&sliced, &read_col_indices, sel)?;
            result.push(projected);
        } else {
            result.push(sliced);
        }
    }

    Ok(result)
}

pub fn count_parquet_filtered(
    path: &str,
    predicate: &Predicate,
    schema: &SchemaRef,
    bloom: Option<&crate::bloom_index::BloomIndex>,
    roaring: Option<&crate::roaring_index::RoaringIndex>,
    cached_metadata: Option<Arc<ParquetMetaData>>,
    mark_rgs: Option<&[usize]>,
) -> Result<u64, EngineError> {
    let options = ArrowReaderOptions::new().with_page_index(page_index_enabled());
    let file = File::open(path)?;
    let (builder, metadata) = if let Some(m) = cached_metadata {
        match ArrowReaderMetadata::try_new(m.clone(), options) {
            Ok(arm) => {
                let b = ParquetRecordBatchReaderBuilder::new_with_metadata(file, arm);
                (b, m)
            }
            Err(_) => {
                drop(file);
                let f2 = File::open(path)?;
                let b = ParquetRecordBatchReaderBuilder::try_new_with_options(
                    f2,
                    ArrowReaderOptions::new().with_page_index(page_index_enabled()),
                )?;
                let md = b.metadata().clone();
                (b, md)
            }
        }
    } else {
        let b = ParquetRecordBatchReaderBuilder::try_new_with_options(
            file,
            ArrowReaderOptions::new().with_page_index(page_index_enabled()),
        )?;
        let md = b.metadata().clone();
        (b, md)
    };
    let n_rgs = metadata.num_row_groups();

    let qualifying_rgs: Vec<usize> = if let Some(ri) = roaring {
        if let Some((col_name, values)) = crate::roaring_index::applicable_predicate(predicate) {
            if schema.index_of(col_name).is_ok() {
                if values.len() == 1 {
                    ri.row_groups_for_eq(values[0], n_rgs)
                        .unwrap_or_else(|| (0..n_rgs).collect())
                } else {
                    ri.row_groups_for_in(&values, n_rgs)
                }
            } else {
                (0..n_rgs)
                    .filter(|&i| {
                        !can_prune_row_group(
                            predicate,
                            metadata.row_group(i),
                            schema,
                            bloom.map(|b| (b, i)),
                        )
                    })
                    .collect()
            }
        } else {
            (0..n_rgs)
                .filter(|&i| {
                    !can_prune_row_group(
                        predicate,
                        metadata.row_group(i),
                        schema,
                        bloom.map(|b| (b, i)),
                    )
                })
                .collect()
        }
    } else {
        (0..n_rgs)
            .filter(|&i| {
                !can_prune_row_group(
                    predicate,
                    metadata.row_group(i),
                    schema,
                    bloom.map(|b| (b, i)),
                )
            })
            .collect()
    };
    let qualifying_rgs: Vec<usize> = if let Some(mark) = mark_rgs {
        let mark_set: std::collections::HashSet<usize> = mark.iter().copied().collect();
        qualifying_rgs
            .into_iter()
            .filter(|i| mark_set.contains(i))
            .collect()
    } else {
        qualifying_rgs
    };

    if qualifying_rgs.is_empty() {
        return Ok(0);
    }

    let pred_col_indices = predicate_column_indices(predicate, schema);
    let pred_mask1 =
        ProjectionMask::roots(builder.parquet_schema(), pred_col_indices.iter().copied());
    let pred_mask2 =
        ProjectionMask::roots(builder.parquet_schema(), pred_col_indices.iter().copied());

    let cloned = predicate.clone();
    let row_filter = RowFilter::new(vec![Box::new(ArrowPredicateFn::new(
        pred_mask1,
        move |batch| {
            predicate_to_bool_array(&batch, &cloned)
                .map_err(|e| arrow::error::ArrowError::ExternalError(Box::new(e)))
        },
    ))]);

    let reader = builder
        .with_row_groups(qualifying_rgs)
        .with_projection(pred_mask2)
        .with_row_filter(row_filter)
        .build()?;

    let mut count = 0u64;
    for batch_result in reader {
        let batch = batch_result.map_err(EngineError::Arrow)?;
        count += batch.num_rows() as u64;
    }

    Ok(count)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RowGroupColumnStat {
    pub rg_index: usize,
    pub row_offset: u64,
    pub row_count: u64,
    pub null_count: u64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
}

pub fn parquet_row_group_column_stats(
    path: &str,
    col_idx: usize,
) -> Result<Vec<RowGroupColumnStat>, EngineError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let metadata = builder.metadata();

    let mut row_offset = 0u64;
    let mut results = Vec::new();

    for rg_idx in 0..metadata.num_row_groups() {
        let rg = metadata.row_group(rg_idx);
        let row_count = rg.num_rows() as u64;

        let (min, max, null_count) = if col_idx < rg.num_columns() {
            let col_meta = rg.column(col_idx);
            if let Some(stats) = col_meta.statistics() {
                let nc = stats.null_count_opt().unwrap_or(0);
                let (mn, mx) = statistics_to_f64(stats);
                (mn, mx, nc)
            } else {
                (None, None, 0)
            }
        } else {
            (None, None, 0)
        };

        let mean = match (min, max) {
            (Some(mn), Some(mx)) => Some((mn + mx) / 2.0),
            _ => None,
        };

        results.push(RowGroupColumnStat {
            rg_index: rg_idx,
            row_offset,
            row_count,
            null_count,
            min,
            max,
            mean,
        });

        row_offset += row_count;
    }

    Ok(results)
}

pub fn parquet_column_stats_from_metadata(
    path: &str,
    col_idx: usize,
) -> Result<(Option<f64>, Option<f64>, u64, u64), EngineError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let metadata = builder.metadata();

    let total_rows = metadata.file_metadata().num_rows() as u64;
    let mut global_min: Option<f64> = None;
    let mut global_max: Option<f64> = None;
    let mut total_null_count = 0u64;

    for rg_idx in 0..metadata.num_row_groups() {
        let rg = metadata.row_group(rg_idx);
        if col_idx >= rg.num_columns() {
            continue;
        }
        let col_meta = rg.column(col_idx);
        if let Some(stats) = col_meta.statistics() {
            total_null_count += stats.null_count_opt().unwrap_or(0);
            let (min, max) = statistics_to_f64(stats);
            if let Some(m) = min {
                global_min = Some(global_min.map(|g: f64| g.min(m)).unwrap_or(m));
            }
            if let Some(m) = max {
                global_max = Some(global_max.map(|g: f64| g.max(m)).unwrap_or(m));
            }
        }
    }

    Ok((global_min, global_max, total_null_count, total_rows))
}

pub fn parquet_all_quick_stats(path: &str) -> Result<Vec<ColMinMaxNull>, EngineError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let metadata = builder.metadata();

    let n_cols = metadata.file_metadata().schema_descr().num_columns();
    let mut mins: Vec<Option<f64>> = vec![None; n_cols];
    let mut maxs: Vec<Option<f64>> = vec![None; n_cols];
    let mut null_counts: Vec<u64> = vec![0; n_cols];

    for rg_idx in 0..metadata.num_row_groups() {
        let rg = metadata.row_group(rg_idx);
        let rg_cols = rg.num_columns();
        for col_idx in 0..n_cols.min(rg_cols) {
            let col_meta = rg.column(col_idx);
            if let Some(stats) = col_meta.statistics() {
                null_counts[col_idx] += stats.null_count_opt().unwrap_or(0);
                let (mn, mx) = statistics_to_f64(stats);
                if let Some(m) = mn {
                    mins[col_idx] = Some(mins[col_idx].map(|g: f64| g.min(m)).unwrap_or(m));
                }
                if let Some(m) = mx {
                    maxs[col_idx] = Some(maxs[col_idx].map(|g: f64| g.max(m)).unwrap_or(m));
                }
            }
        }
    }

    Ok((0..n_cols)
        .map(|i| (mins[i], maxs[i], null_counts[i]))
        .collect())
}

pub fn load_parquet_metadata(path: &str) -> Result<ArrowReaderMetadata, EngineError> {
    let file = File::open(path)?;
    ArrowReaderMetadata::load(&file, ArrowReaderOptions::new()).map_err(EngineError::Parquet)
}

pub fn read_parquet_row_group_column(
    path: &str,
    col_idx: usize,
    rg_idx: usize,
    metadata: &ArrowReaderMetadata,
) -> Result<Vec<RecordBatch>, EngineError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::new_with_metadata(file, metadata.clone());
    let parquet_schema = builder.parquet_schema();
    let mask = ProjectionMask::roots(parquet_schema, [col_idx]);
    let reader = builder
        .with_projection(mask)
        .with_row_groups(vec![rg_idx])
        .build()
        .map_err(EngineError::Parquet)?;
    reader
        .collect::<Result<Vec<_>, _>>()
        .map_err(EngineError::Arrow)
}

pub fn can_prune_row_group(
    predicate: &Predicate,
    rg: &RowGroupMetaData,
    schema: &SchemaRef,
    bloom: Option<(&crate::bloom_index::BloomIndex, usize)>,
) -> bool {
    match predicate {
        Predicate::Gt { column, value } => {
            if let Some((min, max, _, _)) = rg_col_key_bounds(column, rg, schema) {
                match (value, &max) {
                    (Literal::Int(v), RgKeyBound::Float(m)) => return *m <= *v as f64,
                    (Literal::Float(v), RgKeyBound::Float(m)) => return *m <= *v,
                    (Literal::Text(v), RgKeyBound::Text(m)) => return m.as_str() <= v.as_str(),
                    _ => {}
                }
                let _ = min;
            }
            false
        }
        Predicate::Gte { column, value } => {
            if let Some((min, max, _, _)) = rg_col_key_bounds(column, rg, schema) {
                match (value, &max) {
                    (Literal::Int(v), RgKeyBound::Float(m)) => return *m < *v as f64,
                    (Literal::Float(v), RgKeyBound::Float(m)) => return *m < *v,
                    (Literal::Text(v), RgKeyBound::Text(m)) => return m.as_str() < v.as_str(),
                    _ => {}
                }
                let _ = min;
            }
            false
        }
        Predicate::Lt { column, value } => {
            if let Some((min, _, _, _)) = rg_col_key_bounds(column, rg, schema) {
                match (value, &min) {
                    (Literal::Int(v), RgKeyBound::Float(m)) => return *m >= *v as f64,
                    (Literal::Float(v), RgKeyBound::Float(m)) => return *m >= *v,
                    (Literal::Text(v), RgKeyBound::Text(m)) => return m.as_str() >= v.as_str(),
                    _ => {}
                }
            }
            false
        }
        Predicate::Lte { column, value } => {
            if let Some((min, _, _, _)) = rg_col_key_bounds(column, rg, schema) {
                match (value, &min) {
                    (Literal::Int(v), RgKeyBound::Float(m)) => return *m > *v as f64,
                    (Literal::Float(v), RgKeyBound::Float(m)) => return *m > *v,
                    (Literal::Text(v), RgKeyBound::Text(m)) => return m.as_str() > v.as_str(),
                    _ => {}
                }
            }
            false
        }
        Predicate::Eq { column, value } => {
            if let Some((min, max, _, _)) = rg_col_key_bounds(column, rg, schema) {
                let prunable = match (value, &min, &max) {
                    (Literal::Int(v), RgKeyBound::Float(lo), RgKeyBound::Float(hi)) => {
                        let vf = *v as f64;
                        vf < *lo || vf > *hi
                    }
                    (Literal::Float(v), RgKeyBound::Float(lo), RgKeyBound::Float(hi)) => {
                        *v < *lo || *v > *hi
                    }
                    (Literal::Text(v), RgKeyBound::Text(lo), RgKeyBound::Text(hi)) => {
                        v.as_str() < lo.as_str() || v.as_str() > hi.as_str()
                    }
                    _ => false,
                };
                if prunable {
                    return true;
                }
            }
            if let Some((bloom_idx, rg_idx)) = bloom {
                return crate::bloom_index::can_prune_row_group_with_bloom(
                    bloom_idx, rg_idx, predicate, schema,
                );
            }
            false
        }
        Predicate::Between { column, lo, hi } => {
            if let Some((min, max, _, _)) = rg_col_key_bounds(column, rg, schema) {
                match (lo, hi, &min, &max) {
                    (
                        Literal::Int(lv),
                        Literal::Int(hv),
                        RgKeyBound::Float(rg_min),
                        RgKeyBound::Float(rg_max),
                    ) => {
                        return *rg_max < *lv as f64 || *rg_min > *hv as f64;
                    }
                    (
                        Literal::Float(lv),
                        Literal::Float(hv),
                        RgKeyBound::Float(rg_min),
                        RgKeyBound::Float(rg_max),
                    ) => {
                        return *rg_max < *lv || *rg_min > *hv;
                    }
                    (
                        Literal::Text(lv),
                        Literal::Text(hv),
                        RgKeyBound::Text(rg_min),
                        RgKeyBound::Text(rg_max),
                    ) => {
                        return rg_max.as_str() < lv.as_str() || rg_min.as_str() > hv.as_str();
                    }
                    _ => {}
                }
            }
            false
        }
        Predicate::In { column, values: _ } => {
            if let Some((bloom_idx, rg_idx)) = bloom {
                if let Ok(col_idx) = schema.index_of(column) {
                    let _ = col_idx;
                    return crate::bloom_index::can_prune_row_group_with_bloom(
                        bloom_idx, rg_idx, predicate, schema,
                    );
                }
            }
            false
        }
        Predicate::IsNull { column } => {
            let (_, _, null_count, _) = match rg_col_stats(column, rg, schema) {
                Some(s) => s,
                None => return false,
            };
            null_count.map(|nc| nc == 0).unwrap_or(false)
        }
        Predicate::IsNotNull { column } => {
            let (_, _, null_count, num_rows) = match rg_col_stats(column, rg, schema) {
                Some(s) => s,
                None => return false,
            };
            match null_count {
                Some(nc) => nc == num_rows,
                None => false,
            }
        }
        Predicate::And { exprs } => exprs
            .iter()
            .any(|e| can_prune_row_group(e, rg, schema, bloom)),
        Predicate::Or { exprs } => exprs
            .iter()
            .all(|e| can_prune_row_group(e, rg, schema, bloom)),
        _ => false,
    }
}

fn rg_col_stats(col_name: &str, rg: &RowGroupMetaData, schema: &SchemaRef) -> Option<RgColStats> {
    let col_idx = schema.index_of(col_name).ok()?;
    if col_idx >= rg.num_columns() {
        return None;
    }
    let col_meta = rg.column(col_idx);
    let num_rows = rg.num_rows() as u64;
    let stats = col_meta.statistics()?;
    let null_count = stats.null_count_opt();
    let (min, max) = statistics_to_f64(stats);
    Some((min, max, null_count, num_rows))
}

fn statistics_to_f64(stats: &Statistics) -> (Option<f64>, Option<f64>) {
    match stats {
        Statistics::Int32(v) => (
            v.min_opt().map(|&x| x as f64),
            v.max_opt().map(|&x| x as f64),
        ),
        Statistics::Int64(v) => (
            v.min_opt().map(|&x| x as f64),
            v.max_opt().map(|&x| x as f64),
        ),
        Statistics::Float(v) => (
            v.min_opt().map(|&x| x as f64),
            v.max_opt().map(|&x| x as f64),
        ),
        Statistics::Double(v) => (v.min_opt().copied(), v.max_opt().copied()),
        _ => (None, None),
    }
}

pub fn read_filtered_tile_multifile(
    files: &[String],
    predicate: &Predicate,
    schema: &SchemaRef,
    ops: &[tv_core::ViewOp],
    row_offset: usize,
    row_limit: usize,
) -> Result<Vec<RecordBatch>, EngineError> {
    use crate::executor::execute_pipeline_skip_filter;

    let mut global_matched = 0usize;
    let mut collected = 0usize;
    let mut result: Vec<RecordBatch> = Vec::new();

    for path in files {
        if collected >= row_limit {
            break;
        }
        let file = std::fs::File::open(path)?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
        let metadata = builder.metadata().clone();
        let n_rgs = metadata.num_row_groups();

        let qualifying_rgs: Vec<usize> = (0..n_rgs)
            .filter(|&i| !can_prune_row_group(predicate, metadata.row_group(i), schema, None))
            .collect();

        if qualifying_rgs.is_empty() {
            continue;
        }

        let pred_col_indices = predicate_column_indices(predicate, schema);
        let pred_mask =
            ProjectionMask::roots(builder.parquet_schema(), pred_col_indices.iter().copied());
        let cloned = predicate.clone();
        let row_filter = RowFilter::new(vec![Box::new(ArrowPredicateFn::new(
            pred_mask,
            move |batch| {
                predicate_to_bool_array(&batch, &cloned)
                    .map_err(|e| arrow::error::ArrowError::ExternalError(Box::new(e)))
            },
        ))]);

        let reader = builder
            .with_row_groups(qualifying_rgs)
            .with_row_filter(row_filter)
            .build()?;

        for batch_result in reader {
            if collected >= row_limit {
                break;
            }
            let batch = batch_result.map_err(EngineError::Arrow)?;
            if batch.num_rows() == 0 {
                continue;
            }
            let n = batch.num_rows();
            if global_matched + n <= row_offset {
                global_matched += n;
                continue;
            }
            let start = row_offset.saturating_sub(global_matched);
            let available = n - start;
            let take = available.min(row_limit - collected);
            let sliced = batch.slice(start, take);
            let processed = execute_pipeline_skip_filter(vec![sliced], ops)?;
            for b in processed {
                if b.num_rows() > 0 {
                    collected += b.num_rows();
                    result.push(b);
                }
            }
            global_matched += n;
        }
    }

    Ok(result)
}

pub fn csv_schema_and_data(path: &str) -> Result<(SchemaRef, Vec<RecordBatch>), EngineError> {
    let file = File::open(path)?;
    let buf = BufReader::new(file);

    let format = arrow_csv::reader::Format::default().with_header(true);
    let mut peek = BufReader::new(File::open(path)?);
    let (schema, _) = format
        .infer_schema(&mut peek, Some(1024))
        .map_err(|e| EngineError::Query(e.to_string()))?;
    let schema = Arc::new(schema);

    let reader = arrow_csv::ReaderBuilder::new(schema.clone())
        .with_header(true)
        .build(buf)
        .map_err(|e| EngineError::Query(e.to_string()))?;

    let batches: Vec<RecordBatch> = reader
        .collect::<Result<_, _>>()
        .map_err(EngineError::Arrow)?;

    Ok((schema, batches))
}

pub fn json_schema_and_data(path: &str) -> Result<(SchemaRef, Vec<RecordBatch>), EngineError> {
    let peek = BufReader::new(File::open(path)?);
    let (schema, _) = arrow_json::reader::infer_json_schema(peek, Some(1024))
        .map_err(|e| EngineError::Query(e.to_string()))?;
    let schema = Arc::new(schema);

    let file = File::open(path)?;
    let buf = BufReader::new(file);
    let reader = arrow_json::ReaderBuilder::new(schema.clone())
        .build(buf)
        .map_err(|e| EngineError::Query(e.to_string()))?;

    let batches: Vec<RecordBatch> = reader
        .collect::<Result<_, _>>()
        .map_err(EngineError::Arrow)?;

    Ok((schema, batches))
}

pub fn arrow_ipc_schema_and_data(path: &str) -> Result<(SchemaRef, Vec<RecordBatch>), EngineError> {
    let file = File::open(path)?;
    let reader = arrow::ipc::reader::FileReader::try_new(file, None)?;
    let schema = reader.schema();
    let batches: Vec<RecordBatch> = reader
        .collect::<Result<_, _>>()
        .map_err(EngineError::Arrow)?;
    Ok((schema, batches))
}

pub fn read_source_full(
    uri: &str,
    format: &SourceFormat,
    col_indices: Option<&[usize]>,
) -> Result<Vec<RecordBatch>, EngineError> {
    match format {
        SourceFormat::Parquet => read_parquet_full(uri, col_indices),
        SourceFormat::Csv => {
            let (_, batches) = csv_schema_and_data(uri)?;
            Ok(batches)
        }
        SourceFormat::Json => {
            let (_, batches) = json_schema_and_data(uri)?;
            Ok(batches)
        }
        SourceFormat::Arrow => {
            let (_, batches) = arrow_ipc_schema_and_data(uri)?;
            Ok(batches)
        }
        other => Err(EngineError::UnsupportedFormat(format!("{other:?}"))),
    }
}

fn build_object_store(uri: &str) -> Result<(Arc<dyn ObjectStore>, ObjPath), EngineError> {
    let parsed = Url::parse(uri).map_err(|e| EngineError::Query(format!("invalid URI: {e}")))?;
    let scheme = parsed.scheme();

    match scheme {
        "s3" | "s3a" => {
            let bucket = parsed
                .host_str()
                .ok_or_else(|| EngineError::Query("S3 URI missing bucket".into()))?;
            let key = parsed.path().trim_start_matches('/');
            let store = AmazonS3Builder::from_env()
                .with_bucket_name(bucket)
                .build()
                .map_err(|e| EngineError::Query(format!("S3 init error: {e}")))?;
            Ok((Arc::new(store), ObjPath::from(key)))
        }
        "gs" | "gcs" => {
            let bucket = parsed
                .host_str()
                .ok_or_else(|| EngineError::Query("GCS URI missing bucket".into()))?;
            let key = parsed.path().trim_start_matches('/');
            let store = GoogleCloudStorageBuilder::from_env()
                .with_bucket_name(bucket)
                .build()
                .map_err(|e| EngineError::Query(format!("GCS init error: {e}")))?;
            Ok((Arc::new(store), ObjPath::from(key)))
        }
        "az" | "abfs" => {
            let account = parsed
                .host_str()
                .ok_or_else(|| EngineError::Query("Azure URI missing account".into()))?;
            let path = parsed.path().trim_start_matches('/');
            let (container, blob_path) = path.split_once('/').unwrap_or((path, ""));
            let store = MicrosoftAzureBuilder::from_env()
                .with_account(account)
                .with_container_name(container)
                .build()
                .map_err(|e| EngineError::Query(format!("Azure init error: {e}")))?;
            Ok((Arc::new(store), ObjPath::from(blob_path)))
        }
        "http" | "https" => {
            let mut base = parsed.clone();
            base.set_path("");
            base.set_query(None);
            base.set_fragment(None);
            let store = HttpBuilder::new()
                .with_url(base.as_str())
                .build()
                .map_err(|e| EngineError::Query(format!("HTTP store init error: {e}")))?;
            let path = parsed.path().trim_start_matches('/');
            Ok((Arc::new(store), ObjPath::from(path)))
        }
        other => Err(EngineError::UnsupportedFormat(format!(
            "unsupported URI scheme: {other}"
        ))),
    }
}

pub async fn parquet_schema_and_rows_cloud(uri: &str) -> Result<(SchemaRef, u64), EngineError> {
    let (store, path) = build_object_store(uri)?;
    let meta = store
        .head(&path)
        .await
        .map_err(|e| EngineError::Query(format!("object store head error: {e}")))?;
    let reader = ParquetObjectReader::new(store, path).with_file_size(meta.size);
    let builder = parquet::arrow::async_reader::ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(EngineError::Parquet)?;
    let schema = builder.schema().clone();
    let n_rows = builder.metadata().file_metadata().num_rows() as u64;
    Ok((schema, n_rows))
}

pub async fn parquet_schema_and_rows_ranged(
    store: Arc<dyn ObjectStore>,
    path: ObjPath,
    file_size: u64,
) -> Result<(SchemaRef, u64), EngineError> {
    let reader = ParquetObjectReader::new(store, path).with_file_size(file_size);
    let builder = parquet::arrow::async_reader::ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(EngineError::Parquet)?;
    let schema = builder.schema().clone();
    let n_rows = builder.metadata().file_metadata().num_rows() as u64;
    Ok((schema, n_rows))
}

pub async fn read_cloud_parquet_full(
    uri: &str,
    col_indices: Option<&[usize]>,
) -> Result<Vec<RecordBatch>, EngineError> {
    let (store, path) = build_object_store(uri)?;
    let reader = ParquetObjectReader::new(store, path);
    let builder = parquet::arrow::async_reader::ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(EngineError::Parquet)?;

    let stream = if let Some(cols) = col_indices {
        let parquet_schema = builder.parquet_schema();
        let mask = ProjectionMask::roots(parquet_schema, cols.iter().copied());
        builder.with_projection(mask).build()?
    } else {
        builder.build()?
    };

    use futures::TryStreamExt;
    stream
        .try_collect::<Vec<_>>()
        .await
        .map_err(EngineError::Parquet)
}

pub async fn read_cloud_parquet_tile(
    uri: &str,
    row_offset: usize,
    col_indices: &[usize],
    rows: usize,
) -> Result<Vec<RecordBatch>, EngineError> {
    let (store, path) = build_object_store(uri)?;
    let reader = ParquetObjectReader::new(store, path);
    let builder = parquet::arrow::async_reader::ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(EngineError::Parquet)?;

    let parquet_schema = builder.parquet_schema();
    let mask = ProjectionMask::roots(parquet_schema, col_indices.iter().copied());
    let stream = builder
        .with_projection(mask)
        .with_offset(row_offset)
        .with_limit(rows)
        .build()?;

    use futures::TryStreamExt;
    stream
        .try_collect::<Vec<_>>()
        .await
        .map_err(EngineError::Parquet)
}

pub async fn list_cloud_parquet_files(uri: &str) -> Result<Vec<String>, EngineError> {
    let parsed = Url::parse(uri).map_err(|e| EngineError::Query(format!("invalid URI: {e}")))?;
    let scheme = parsed.scheme().to_string();
    let bucket = parsed
        .host_str()
        .ok_or_else(|| EngineError::Query("URI missing bucket/host".into()))?
        .to_string();

    let root_uri = format!("{}://{}/", scheme, bucket);
    let (store, _) = build_object_store(&root_uri)?;

    let prefix = parsed.path().trim_start_matches('/');
    let prefix_path = ObjPath::from(prefix);

    use futures::TryStreamExt;
    let objects: Vec<_> = store
        .list(Some(&prefix_path))
        .try_collect()
        .await
        .map_err(|e| EngineError::Query(format!("listing failed: {e}")))?;

    let mut files: Vec<String> = objects
        .into_iter()
        .filter(|o| o.location.as_ref().ends_with(".parquet"))
        .map(|o| format!("{}://{}/{}", scheme, bucket, o.location))
        .collect();
    files.sort();

    Ok(files)
}

pub fn expand_local_glob(pattern: &str) -> Result<Vec<String>, EngineError> {
    let paths = glob::glob(pattern)
        .map_err(|e| EngineError::Query(format!("invalid glob pattern: {e}")))?;
    let mut files: Vec<String> = paths
        .filter_map(|entry| entry.ok())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e == "parquet")
                    .unwrap_or(false)
        })
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    files.sort();
    Ok(files)
}

pub fn predicate_column_indices(predicate: &Predicate, schema: &SchemaRef) -> Vec<usize> {
    let mut cols = std::collections::HashSet::new();
    collect_predicate_columns(predicate, &mut cols);
    cols.into_iter()
        .filter_map(|name| schema.index_of(&name).ok())
        .collect()
}

fn collect_predicate_columns(predicate: &Predicate, cols: &mut std::collections::HashSet<String>) {
    match predicate {
        Predicate::IsNull { column }
        | Predicate::IsNotNull { column }
        | Predicate::Eq { column, .. }
        | Predicate::Ne { column, .. }
        | Predicate::Gt { column, .. }
        | Predicate::Gte { column, .. }
        | Predicate::Lt { column, .. }
        | Predicate::Lte { column, .. }
        | Predicate::Between { column, .. }
        | Predicate::In { column, .. }
        | Predicate::NotIn { column, .. }
        | Predicate::Contains { column, .. }
        | Predicate::StartsWith { column, .. }
        | Predicate::EndsWith { column, .. }
        | Predicate::Regex { column, .. } => {
            cols.insert(column.clone());
        }
        Predicate::And { exprs } | Predicate::Or { exprs } => {
            for e in exprs {
                collect_predicate_columns(e, cols);
            }
        }
        Predicate::Not { expr } => collect_predicate_columns(expr, cols),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{int_string_batch, write_multi_rg_parquet, write_test_parquet};
    use tempfile::TempDir;

    fn make_dir() -> TempDir {
        tempfile::TempDir::new().unwrap()
    }

    fn total_rows(batches: &[RecordBatch]) -> usize {
        batches.iter().map(|b| b.num_rows()).sum()
    }

    #[test]
    fn read_parquet_schema_and_rows() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let path = write_test_parquet(&dir, "test.parquet", &[batch]);
        let (schema, n_rows) = parquet_schema_and_rows(path.to_str().unwrap()).unwrap();
        assert_eq!(n_rows, 100);
        assert_eq!(schema.fields().len(), 4);
    }

    #[test]
    fn read_parquet_tile_offset_limit() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let path = write_test_parquet(&dir, "test.parquet", &[batch]);
        let batches = read_parquet_tile(path.to_str().unwrap(), 10, &[0, 1], 20, None).unwrap();
        assert_eq!(total_rows(&batches), 20);
        let arr = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .unwrap();
        assert_eq!(arr.value(0), 10);
    }

    #[test]
    fn read_parquet_tile_col_projection() {
        let dir = make_dir();
        let batch = int_string_batch(50);
        let path = write_test_parquet(&dir, "test.parquet", &[batch]);
        let batches = read_parquet_tile(path.to_str().unwrap(), 0, &[0], 10, None).unwrap();
        assert_eq!(batches[0].num_columns(), 1);
    }

    #[test]
    fn test_read_parquet_full_all_columns() {
        let dir = make_dir();
        let batch = int_string_batch(30);
        let path = write_test_parquet(&dir, "test.parquet", &[batch]);
        let batches = read_parquet_full(path.to_str().unwrap(), None).unwrap();
        assert_eq!(total_rows(&batches), 30);
        assert_eq!(batches[0].num_columns(), 4);
    }

    #[test]
    fn test_read_parquet_full_with_col_indices() {
        let dir = make_dir();
        let batch = int_string_batch(20);
        let path = write_test_parquet(&dir, "test.parquet", &[batch]);
        let batches = read_parquet_full(path.to_str().unwrap(), Some(&[2])).unwrap();
        assert_eq!(batches[0].num_columns(), 1);
        assert_eq!(batches[0].schema().field(0).name(), "score");
    }

    #[test]
    fn test_read_parquet_full_with_filter() {
        let dir = make_dir();
        let batch = int_string_batch(50);
        let schema = batch.schema();
        let path = write_test_parquet(&dir, "test.parquet", &[batch]);
        let predicate = Predicate::Lt {
            column: "id".into(),
            value: Literal::Int(10),
        };
        let batches =
            read_parquet_full_with_filter(path.to_str().unwrap(), &predicate, &schema).unwrap();
        assert_eq!(total_rows(&batches), 10);
    }

    #[test]
    fn test_read_parquet_filtered_tile_basic() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let schema = batch.schema();
        let path = write_multi_rg_parquet(&dir, "test.parquet", &[batch], 25);
        let predicate = Predicate::Lt {
            column: "id".into(),
            value: Literal::Int(50),
        };
        let batches = read_parquet_filtered_tile(
            path.to_str().unwrap(),
            &predicate,
            &schema,
            None,
            None,
            0,
            20,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(total_rows(&batches) <= 20);
        for batch in &batches {
            let arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::Int64Array>()
                .unwrap();
            for i in 0..arr.len() {
                assert!(arr.value(i) < 50);
            }
        }
    }

    #[test]
    fn test_count_parquet_filtered_basic() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let schema = batch.schema();
        let path = write_test_parquet(&dir, "test.parquet", &[batch]);
        let predicate = Predicate::Lt {
            column: "id".into(),
            value: Literal::Int(30),
        };
        let count = count_parquet_filtered(
            path.to_str().unwrap(),
            &predicate,
            &schema,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(count, 30);
    }

    #[test]
    fn test_build_filter_rg_index_cumulative() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let schema = batch.schema();
        let path = write_multi_rg_parquet(&dir, "test.parquet", &[batch], 25);
        let predicate = Predicate::Gte {
            column: "id".into(),
            value: Literal::Int(0),
        };
        let cumulative = build_filter_rg_index(
            path.to_str().unwrap(),
            &predicate,
            &schema,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(!cumulative.is_empty());
        for i in 1..cumulative.len() {
            assert!(cumulative[i] >= cumulative[i - 1]);
        }
    }

    #[test]
    fn test_parquet_column_stats_from_metadata_basic() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let path = write_test_parquet(&dir, "test.parquet", &[batch]);
        let (min, max, null_count, total_rows) =
            parquet_column_stats_from_metadata(path.to_str().unwrap(), 0).unwrap();
        assert_eq!(total_rows, 100);
        assert_eq!(null_count, 0);
        assert!(min.is_some());
        assert!(max.is_some());
        assert!((min.unwrap() - 0.0).abs() < 1.0);
        assert!((max.unwrap() - 99.0).abs() < 1.0);
    }

    #[test]
    fn test_predicate_column_indices_basic() {
        let dir = make_dir();
        let batch = int_string_batch(10);
        let _ = batch.schema();
        let path = write_test_parquet(&dir, "test.parquet", &[batch]);
        let (schema_ref, _) = parquet_schema_and_rows(path.to_str().unwrap()).unwrap();
        let pred = Predicate::Eq {
            column: "id".into(),
            value: Literal::Int(1),
        };
        let indices = predicate_column_indices(&pred, &schema_ref);
        assert!(indices.contains(&0));
    }
}
