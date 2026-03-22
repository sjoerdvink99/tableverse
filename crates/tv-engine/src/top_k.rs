use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs::File;

use arrow::array::{Array, Float64Array, StringArray};
use arrow::compute::cast;
use arrow::datatypes::{DataType as ArrowDataType, SchemaRef};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ProjectionMask;
use parquet::file::statistics::Statistics;
use tv_core::{SortKey, ViewOp};

use crate::batch_stream::BatchStream;
use crate::error::EngineError;
use crate::executor::{apply_sort, execute_pipeline};

const TOP_K_THRESHOLD: u64 = 10_000;

pub fn top_k_threshold() -> u64 {
    std::env::var("TOP_K_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(TOP_K_THRESHOLD)
}

#[derive(Clone)]
struct RowGroupCandidate {
    rg_idx: usize,
    min_key: SortableKey,
    max_key: SortableKey,
    secondary_min_key: Option<SortableKey>,
    row_start: u64,
    _row_count: u64,
}

#[derive(Clone, Debug)]
enum SortableKey {
    Float(f64),
    Text(String),
    Null,
    Unknown,
}

impl SortableKey {
    fn from_statistics(stats: Option<&Statistics>, descending: bool) -> Self {
        match stats {
            Some(Statistics::Double(s)) => {
                let val = if descending {
                    s.max_opt().copied()
                } else {
                    s.min_opt().copied()
                };
                val.map(SortableKey::Float).unwrap_or(SortableKey::Unknown)
            }
            Some(Statistics::Float(s)) => {
                let val = if descending {
                    s.max_opt().copied().map(|v| v as f64)
                } else {
                    s.min_opt().copied().map(|v| v as f64)
                };
                val.map(SortableKey::Float).unwrap_or(SortableKey::Unknown)
            }
            Some(Statistics::Int32(s)) => {
                let val = if descending {
                    s.max_opt().copied().map(|v| v as f64)
                } else {
                    s.min_opt().copied().map(|v| v as f64)
                };
                val.map(SortableKey::Float).unwrap_or(SortableKey::Unknown)
            }
            Some(Statistics::Int64(s)) => {
                let val = if descending {
                    s.max_opt().copied().map(|v| v as f64)
                } else {
                    s.min_opt().copied().map(|v| v as f64)
                };
                val.map(SortableKey::Float).unwrap_or(SortableKey::Unknown)
            }
            Some(Statistics::ByteArray(s)) => {
                let val = if descending {
                    s.max_opt()
                        .and_then(|b| std::str::from_utf8(b.data()).ok())
                        .map(|s| s.to_string())
                } else {
                    s.min_opt()
                        .and_then(|b| std::str::from_utf8(b.data()).ok())
                        .map(|s| s.to_string())
                };
                val.map(SortableKey::Text).unwrap_or(SortableKey::Unknown)
            }
            _ => SortableKey::Unknown,
        }
    }

    fn compare(&self, other: &Self, descending: bool) -> Ordering {
        let base = match (self, other) {
            (SortableKey::Float(a), SortableKey::Float(b)) => {
                a.partial_cmp(b).unwrap_or(Ordering::Equal)
            }
            (SortableKey::Text(a), SortableKey::Text(b)) => a.cmp(b),
            (SortableKey::Null, SortableKey::Null) => Ordering::Equal,
            (SortableKey::Null, _) => Ordering::Less,
            (_, SortableKey::Null) => Ordering::Greater,
            _ => Ordering::Equal,
        };
        if descending {
            base.reverse()
        } else {
            base
        }
    }
}

impl PartialEq for SortableKey {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other, false) == Ordering::Equal
    }
}

impl Eq for SortableKey {}

struct HeapItem {
    key: SortableKey,
    row_id: u64,
    descending: bool,
}

impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.key.compare(&other.key, false) == Ordering::Equal && self.row_id == other.row_id
    }
}

impl Eq for HeapItem {}

impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        let key_ord = self.key.compare(&other.key, self.descending);
        if key_ord != Ordering::Equal {
            return key_ord.reverse();
        }
        if self.descending {
            other.row_id.cmp(&self.row_id)
        } else {
            self.row_id.cmp(&other.row_id).reverse()
        }
    }
}

struct RgHeapItem {
    candidate: RowGroupCandidate,
    descending: bool,
}

impl PartialEq for RgHeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.candidate.rg_idx == other.candidate.rg_idx
    }
}

impl Eq for RgHeapItem {}

impl PartialOrd for RgHeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RgHeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .candidate
            .min_key
            .compare(&self.candidate.min_key, self.descending)
    }
}

pub fn stream_top_k(
    source_path: &str,
    sort_keys: &[SortKey],
    k: usize,
    schema: &SchemaRef,
) -> Result<Vec<RecordBatch>, EngineError> {
    if sort_keys.is_empty() {
        return Ok(vec![]);
    }

    let primary_key = &sort_keys[0];
    let descending = primary_key.descending;

    let key_col_idx = schema.index_of(&primary_key.column).map_err(|_| {
        EngineError::Query(format!("sort key column not found: {}", primary_key.column))
    })?;

    let secondary_key = sort_keys.get(1);
    let secondary_col_idx = secondary_key.and_then(|sk| schema.index_of(&sk.column).ok());

    let file = File::open(source_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let metadata = builder.metadata().clone();

    let mut rg_start = 0u64;
    let mut candidates: Vec<RowGroupCandidate> = Vec::new();

    for rg_idx in 0..metadata.num_row_groups() {
        let rg = metadata.row_group(rg_idx);
        let rg_rows = rg.num_rows() as u64;

        let stats = rg.column(key_col_idx).statistics();
        let min_key = SortableKey::from_statistics(stats, false);
        let max_key = SortableKey::from_statistics(stats, true);

        let secondary_min_key = if let Some(sec_idx) = secondary_col_idx {
            let sec_stats = rg.column(sec_idx).statistics();
            Some(SortableKey::from_statistics(sec_stats, false))
        } else {
            None
        };

        candidates.push(RowGroupCandidate {
            rg_idx,
            min_key,
            max_key,
            secondary_min_key,
            row_start: rg_start,
            _row_count: rg_rows,
        });
        rg_start += rg_rows;
    }

    let mut rg_heap: BinaryHeap<RgHeapItem> = candidates
        .into_iter()
        .map(|c| RgHeapItem {
            candidate: c,
            descending,
        })
        .collect();

    let mut top_heap: BinaryHeap<HeapItem> = BinaryHeap::with_capacity(k + 1);

    while let Some(rg_item) = rg_heap.pop() {
        let candidate = &rg_item.candidate;

        if top_heap.len() >= k {
            let worst = top_heap.peek().unwrap();
            let next_best = if descending {
                &candidate.max_key
            } else {
                &candidate.min_key
            };
            let cmp = next_best.compare(&worst.key, descending);
            if cmp == Ordering::Less {
                break;
            }
            if cmp == Ordering::Equal {
                let has_secondary = candidate.secondary_min_key.is_some();
                if !has_secondary || secondary_key.map(|sk| sk.descending).is_none() {
                    break;
                }
            }
        }

        let rg_batches =
            read_row_group(source_path, rg_item.candidate.rg_idx, key_col_idx, schema)?;

        let mut row_offset = candidate.row_start;
        for batch in &rg_batches {
            let key_col = batch.column(batch.schema().index_of(&primary_key.column).unwrap_or(0));
            let float_col = cast(key_col.as_ref(), &ArrowDataType::Float64).ok();
            let str_col = cast(key_col.as_ref(), &ArrowDataType::Utf8).ok();

            for row_idx in 0..batch.num_rows() {
                let key = if let Some(ref fc) = float_col {
                    if let Some(a) = fc.as_any().downcast_ref::<Float64Array>() {
                        if a.is_null(row_idx) {
                            SortableKey::Null
                        } else {
                            SortableKey::Float(a.value(row_idx))
                        }
                    } else {
                        SortableKey::Unknown
                    }
                } else if let Some(ref sc) = str_col {
                    if let Some(a) = sc.as_any().downcast_ref::<StringArray>() {
                        if a.is_null(row_idx) {
                            SortableKey::Null
                        } else {
                            SortableKey::Text(a.value(row_idx).to_string())
                        }
                    } else {
                        SortableKey::Unknown
                    }
                } else {
                    SortableKey::Unknown
                };

                top_heap.push(HeapItem {
                    key,
                    row_id: row_offset + row_idx as u64,
                    descending,
                });

                if top_heap.len() > k {
                    top_heap.pop();
                }
            }
            row_offset += batch.num_rows() as u64;
        }
    }

    let mut winning_ids: Vec<u64> = top_heap.into_iter().map(|item| item.row_id).collect();
    winning_ids.sort_unstable();

    let col_indices: Vec<usize> = (0..schema.fields().len()).collect();
    let mut batches = crate::sort_index::read_rows_by_ids(source_path, &winning_ids, &col_indices)?;

    batches = apply_sort(batches, sort_keys)?;
    let total: usize = batches.iter().map(|b| b.num_rows()).sum();
    let take_n = total.min(k);
    Ok(slice_batches(&batches, 0, take_n))
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

fn read_row_group(
    source_path: &str,
    rg_idx: usize,
    key_col_idx: usize,
    _schema: &SchemaRef,
) -> Result<Vec<RecordBatch>, EngineError> {
    let file = File::open(source_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let mask = ProjectionMask::roots(builder.parquet_schema(), [key_col_idx]);

    let row_groups = vec![rg_idx];
    let reader = builder
        .with_projection(mask)
        .with_row_groups(row_groups)
        .build()
        .map_err(EngineError::Parquet)?;

    reader
        .collect::<Result<Vec<_>, _>>()
        .map_err(EngineError::Arrow)
}

const TOP_K_CHUNK_BYTES: usize = 64 * 1024 * 1024;

pub fn top_k_from_batches(
    stream: BatchStream,
    stateless_ops: &[ViewOp],
    sort_keys: &[SortKey],
    k: usize,
    schema: &SchemaRef,
) -> Result<Vec<RecordBatch>, EngineError> {
    if sort_keys.is_empty() || k == 0 {
        return Ok(vec![]);
    }

    let mut survivors: Vec<RecordBatch> = Vec::new();
    let mut chunk: Vec<RecordBatch> = Vec::new();
    let mut chunk_bytes: usize = 0;

    let flush_chunk =
        |chunk: Vec<RecordBatch>, survivors: &mut Vec<RecordBatch>| -> Result<(), EngineError> {
            if chunk.is_empty() {
                return Ok(());
            }
            let sorted = apply_sort(chunk, sort_keys)?;
            let total: usize = sorted.iter().map(|b| b.num_rows()).sum();
            let take = total.min(k);
            let trimmed = slice_top_k(&sorted, take);
            survivors.extend(trimmed);
            let combined = apply_sort(survivors.clone(), sort_keys)?;
            let combined_total: usize = combined.iter().map(|b| b.num_rows()).sum();
            let combined_take = combined_total.min(k);
            *survivors = slice_top_k(&combined, combined_take);
            Ok(())
        };

    let _ = schema;

    for batch_result in stream {
        let batch = batch_result?;
        if batch.num_rows() == 0 {
            continue;
        }
        let processed = if stateless_ops.is_empty() {
            vec![batch]
        } else {
            execute_pipeline(vec![batch], stateless_ops)?
        };
        for b in processed {
            if b.num_rows() == 0 {
                continue;
            }
            let bsz: usize = (0..b.num_columns())
                .map(|i| b.column(i).get_array_memory_size())
                .sum();
            chunk_bytes += bsz;
            chunk.push(b);
            if chunk_bytes >= TOP_K_CHUNK_BYTES {
                let c = std::mem::take(&mut chunk);
                chunk_bytes = 0;
                flush_chunk(c, &mut survivors)?;
            }
        }
    }

    if !chunk.is_empty() {
        flush_chunk(chunk, &mut survivors)?;
    }

    let final_sorted = apply_sort(survivors, sort_keys)?;
    let total: usize = final_sorted.iter().map(|b| b.num_rows()).sum();
    let take = total.min(k);
    Ok(slice_top_k(&final_sorted, take))
}

fn slice_top_k(batches: &[RecordBatch], k: usize) -> Vec<RecordBatch> {
    let mut result = Vec::new();
    let mut collected = 0usize;
    for batch in batches {
        if collected >= k {
            break;
        }
        let remaining = k - collected;
        if batch.num_rows() <= remaining {
            result.push(batch.clone());
            collected += batch.num_rows();
        } else {
            result.push(batch.slice(0, remaining));
            collected = k;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{int_string_batch, write_test_parquet};
    use arrow::array::Int64Array;
    use tempfile::TempDir;
    use tv_core::SortKey;

    fn make_dir() -> TempDir {
        tempfile::TempDir::new().unwrap()
    }

    fn schema_for_batch(batch: &arrow::record_batch::RecordBatch) -> arrow::datatypes::SchemaRef {
        batch.schema()
    }

    #[test]
    fn stream_top_k_ascending() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let schema = schema_for_batch(&batch);
        let parquet_path = write_test_parquet(&dir, "data.parquet", &[batch]);

        let sort_keys = vec![SortKey {
            column: "id".to_string(),
            descending: false,
            nulls_last: true,
        }];

        let result = stream_top_k(parquet_path.to_str().unwrap(), &sort_keys, 5, &schema).unwrap();

        let total: usize = result.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total, 5);

        let ids: Vec<i64> = result
            .iter()
            .flat_map(|b| {
                b.column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values()
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
            })
            .collect();

        assert_eq!(ids.len(), 5, "should return exactly 5 rows");
        let mut sorted_ids = ids.clone();
        sorted_ids.sort_unstable();
        assert_eq!(
            ids, sorted_ids,
            "ascending result should be sorted ascending: {ids:?}"
        );
    }

    #[test]
    fn stream_top_k_descending() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let schema = schema_for_batch(&batch);
        let parquet_path = write_test_parquet(&dir, "data.parquet", &[batch]);

        let sort_keys = vec![SortKey {
            column: "id".to_string(),
            descending: true,
            nulls_last: true,
        }];

        let result = stream_top_k(parquet_path.to_str().unwrap(), &sort_keys, 5, &schema).unwrap();

        let total: usize = result.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total, 5);

        let ids: Vec<i64> = result
            .iter()
            .flat_map(|b| {
                b.column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values()
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
            })
            .collect();

        assert_eq!(ids.len(), 5, "should return exactly 5 rows");
        let mut sorted_ids_desc = ids.clone();
        sorted_ids_desc.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(
            ids, sorted_ids_desc,
            "descending result should be sorted descending: {ids:?}"
        );
    }

    #[test]
    fn stream_top_k_k_larger_than_data() {
        let dir = make_dir();
        let batch = int_string_batch(5);
        let schema = schema_for_batch(&batch);
        let parquet_path = write_test_parquet(&dir, "data.parquet", &[batch]);

        let sort_keys = vec![SortKey {
            column: "id".to_string(),
            descending: false,
            nulls_last: true,
        }];

        let result =
            stream_top_k(parquet_path.to_str().unwrap(), &sort_keys, 100, &schema).unwrap();

        let total: usize = result.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total, 5);
    }

    #[test]
    fn sortable_key_compare_ordering() {
        let f1 = SortableKey::Float(1.0);
        let f2 = SortableKey::Float(2.0);
        let t1 = SortableKey::Text("apple".to_string());
        let t2 = SortableKey::Text("banana".to_string());
        let null = SortableKey::Null;

        assert_eq!(f1.compare(&f2, false), std::cmp::Ordering::Less);
        assert_eq!(f2.compare(&f1, false), std::cmp::Ordering::Greater);
        assert_eq!(f1.compare(&f1, false), std::cmp::Ordering::Equal);

        assert_eq!(f1.compare(&f2, true), std::cmp::Ordering::Greater);
        assert_eq!(f2.compare(&f1, true), std::cmp::Ordering::Less);

        assert_eq!(t1.compare(&t2, false), std::cmp::Ordering::Less);
        assert_eq!(t2.compare(&t1, false), std::cmp::Ordering::Greater);

        assert_eq!(null.compare(&f1, false), std::cmp::Ordering::Less);
        assert_eq!(f1.compare(&null, false), std::cmp::Ordering::Greater);
    }
}
