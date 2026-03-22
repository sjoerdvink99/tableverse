use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs::File;

use arrow::array::{Array, ArrayRef, BooleanArray, StringArray};
use arrow::compute::{cast, concat_batches, filter_record_batch, interleave, SortOptions};
use arrow::datatypes::DataType as ArrowDataType;
use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use arrow::row::{RowConverter, Rows, SortField};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use tv_core::SortKey;

use crate::error::EngineError;
use crate::executor::apply_sort;
use crate::spill::{SpillReader, SpillWriter, SpilledRun};

struct MergeEntry {
    key: Vec<u8>,
    cursor_idx: usize,
}

impl PartialEq for MergeEntry {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for MergeEntry {}
impl PartialOrd for MergeEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for MergeEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.key.cmp(&self.key)
    }
}

fn batch_byte_size(batch: &RecordBatch) -> u64 {
    (0..batch.num_columns())
        .map(|i| batch.column(i).get_array_memory_size() as u64)
        .sum()
}

fn sort_chunk_bytes() -> u64 {
    std::env::var("SORT_CHUNK_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(268_435_456)
}

fn max_merge_fanout() -> usize {
    std::env::var("MAX_MERGE_FANOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64)
}

pub struct ExternalSorter {
    pub sort_keys: Vec<SortKey>,
    pub schema: SchemaRef,
}

pub struct SortedResult {
    pub runs: Vec<SpilledRun>,
    pub total_rows: u64,
    pub cumulative_rows: Vec<u64>,
}

struct RunCursor {
    reader: SpillReader,
    batch: RecordBatch,
    row: usize,
    encoded: Rows,
}

impl ExternalSorter {
    pub fn new(sort_keys: Vec<SortKey>, schema: SchemaRef) -> Self {
        Self { sort_keys, schema }
    }

    pub fn sort_to_initial_runs(
        &self,
        stream: impl Iterator<Item = Result<RecordBatch, EngineError>>,
        writer: &mut SpillWriter,
    ) -> Result<(Vec<SpilledRun>, u64), EngineError> {
        let chunk_bytes = sort_chunk_bytes();
        let mut runs: Vec<SpilledRun> = Vec::new();
        let mut chunk: Vec<RecordBatch> = Vec::new();
        let mut chunk_bytes_acc = 0u64;
        let mut total_rows = 0u64;

        for batch_result in stream {
            let batch = batch_result?;
            if batch.num_rows() == 0 {
                continue;
            }
            chunk_bytes_acc += batch_byte_size(&batch);
            total_rows += batch.num_rows() as u64;
            chunk.push(batch);

            if chunk_bytes_acc >= chunk_bytes {
                let sorted = apply_sort(chunk, &self.sort_keys)?;
                runs.push(writer.write_run(&sorted)?);
                chunk = Vec::new();
                chunk_bytes_acc = 0;
            }
        }

        if !chunk.is_empty() {
            let sorted = apply_sort(chunk, &self.sort_keys)?;
            runs.push(writer.write_run(&sorted)?);
        }

        Ok((runs, total_rows))
    }

    pub fn cascade_runs(
        &self,
        runs: Vec<SpilledRun>,
        writer: &mut SpillWriter,
    ) -> Result<Vec<SpilledRun>, EngineError> {
        self.cascade_merge_if_needed(runs, writer, max_merge_fanout())
    }

    pub fn sort_to_runs(
        &self,
        stream: impl Iterator<Item = Result<RecordBatch, EngineError>>,
        writer: &mut SpillWriter,
    ) -> Result<SortedResult, EngineError> {
        let (initial_runs, total_rows) = self.sort_to_initial_runs(stream, writer)?;

        if initial_runs.is_empty() {
            return Ok(SortedResult {
                runs: initial_runs,
                total_rows: 0,
                cumulative_rows: vec![],
            });
        }

        let runs = self.cascade_runs(initial_runs, writer)?;
        let cumulative_rows: Vec<u64> = runs
            .iter()
            .scan(0u64, |acc, r| {
                *acc += r.row_count;
                Some(*acc)
            })
            .collect();
        Ok(SortedResult {
            runs,
            total_rows,
            cumulative_rows,
        })
    }

    fn cascade_merge_if_needed(
        &self,
        mut runs: Vec<SpilledRun>,
        writer: &mut SpillWriter,
        fanout: usize,
    ) -> Result<Vec<SpilledRun>, EngineError> {
        while runs.len() > fanout {
            let mut next_runs: Vec<SpilledRun> = Vec::new();
            let mut i = 0;
            while i < runs.len() {
                let end = (i + fanout).min(runs.len());
                let merged = self.merge_runs_to_run(&runs[i..end], writer)?;
                next_runs.push(merged);
                i = end;
            }
            runs = next_runs;
        }
        Ok(runs)
    }

    fn merge_runs_to_run(
        &self,
        runs: &[SpilledRun],
        writer: &mut SpillWriter,
    ) -> Result<SpilledRun, EngineError> {
        let batches = self.do_merge(runs, 0, u64::MAX)?;
        writer.write_run(&batches)
    }

    pub fn merge_tile(
        &self,
        runs: &[SpilledRun],
        _cumulative_rows: &[u64],
        row_offset: usize,
        row_limit: usize,
    ) -> Result<Vec<RecordBatch>, EngineError> {
        self.do_merge(runs, row_offset as u64, row_limit as u64)
    }

    pub fn merge_all(&self, runs: &[SpilledRun]) -> Result<Vec<RecordBatch>, EngineError> {
        self.do_merge(runs, 0, u64::MAX)
    }

    pub fn merge_dedup_tile(
        &self,
        runs: &[SpilledRun],
        _cumulative_rows: &[u64],
        dedup_columns: &[String],
        row_offset: usize,
        row_limit: usize,
    ) -> Result<Vec<RecordBatch>, EngineError> {
        let all = self.do_merge(runs, 0, u64::MAX)?;
        let deduped = dedup_sorted_batches(all, dedup_columns, &self.schema)?;
        Ok(slice_result(deduped, row_offset, row_limit))
    }

    pub fn merge_to_single_run(
        &self,
        runs: &[SpilledRun],
        writer: &mut SpillWriter,
    ) -> Result<SpilledRun, EngineError> {
        let batches = self.do_merge(runs, 0, u64::MAX)?;
        writer.write_run(&batches)
    }

    fn do_merge(
        &self,
        runs: &[SpilledRun],
        row_offset: u64,
        row_limit: u64,
    ) -> Result<Vec<RecordBatch>, EngineError> {
        if runs.is_empty() {
            return Ok(vec![]);
        }
        if runs.len() == 1 {
            return read_run_slice(&runs[0], row_offset, row_limit);
        }

        let converter = build_row_converter(&self.sort_keys, &self.schema)?;
        let n_runs = runs.len();
        let n_cols = self.schema.fields().len();

        let mut cursors: Vec<Option<RunCursor>> = Vec::with_capacity(n_runs);
        let mut heap: BinaryHeap<MergeEntry> = BinaryHeap::with_capacity(n_runs);

        for (idx, run) in runs.iter().enumerate() {
            let mut reader = SpillReader::open(&run.path)?;
            match reader.next() {
                Some(Ok(batch)) => {
                    let key_cols = extract_key_cols(&batch, &self.sort_keys)?;
                    let encoded = converter.convert_columns(&key_cols)?;
                    let key = encoded.row(0).as_ref().to_vec();
                    cursors.push(Some(RunCursor {
                        reader,
                        batch,
                        row: 0,
                        encoded,
                    }));
                    heap.push(MergeEntry {
                        key,
                        cursor_idx: idx,
                    });
                }
                Some(Err(e)) => return Err(e),
                None => cursors.push(None),
            }
        }

        let mut output_batches: Vec<RecordBatch> = Vec::new();
        let mut pending: Vec<(usize, usize)> = Vec::new();
        let mut skipped: u64 = 0;
        let mut collected: u64 = 0;

        while let Some(MergeEntry {
            cursor_idx: run_idx,
            ..
        }) = heap.pop()
        {
            let cur_row;
            let batch_exhausted;
            {
                let cursor = cursors[run_idx].as_mut().unwrap();
                cur_row = cursor.row;
                cursor.row += 1;
                batch_exhausted = cursor.row >= cursor.batch.num_rows();
            }

            if skipped < row_offset {
                skipped += 1;
            } else if collected < row_limit {
                pending.push((run_idx, cur_row));
                collected += 1;
            }

            if batch_exhausted {
                if !pending.is_empty() {
                    let snapshots: Vec<Option<RecordBatch>> = cursors
                        .iter()
                        .map(|c| c.as_ref().map(|c| c.batch.clone()))
                        .collect();
                    let out = emit_pending(&pending, &snapshots, n_cols, &self.schema)?;
                    if out.num_rows() > 0 {
                        output_batches.push(out);
                    }
                    pending.clear();
                }

                let next = cursors[run_idx].as_mut().unwrap().reader.next();
                match next {
                    Some(Ok(new_batch)) => {
                        let key_cols = extract_key_cols(&new_batch, &self.sort_keys)?;
                        let new_encoded = converter.convert_columns(&key_cols)?;
                        let key = new_encoded.row(0).as_ref().to_vec();
                        let c = cursors[run_idx].as_mut().unwrap();
                        c.batch = new_batch;
                        c.row = 0;
                        c.encoded = new_encoded;
                        heap.push(MergeEntry {
                            key,
                            cursor_idx: run_idx,
                        });
                    }
                    Some(Err(e)) => return Err(e),
                    None => {
                        cursors[run_idx] = None;
                    }
                }
            } else {
                let key = cursors[run_idx]
                    .as_ref()
                    .unwrap()
                    .encoded
                    .row(cursors[run_idx].as_ref().unwrap().row)
                    .as_ref()
                    .to_vec();
                heap.push(MergeEntry {
                    key,
                    cursor_idx: run_idx,
                });
            }

            if collected >= row_limit {
                break;
            }
        }

        if !pending.is_empty() {
            let snapshots: Vec<Option<RecordBatch>> = cursors
                .iter()
                .map(|c| c.as_ref().map(|c| c.batch.clone()))
                .collect();
            let out = emit_pending(&pending, &snapshots, n_cols, &self.schema)?;
            if out.num_rows() > 0 {
                output_batches.push(out);
            }
        }

        Ok(output_batches)
    }
}

fn build_row_converter(
    sort_keys: &[SortKey],
    schema: &SchemaRef,
) -> Result<RowConverter, EngineError> {
    let fields: Vec<SortField> = sort_keys
        .iter()
        .filter_map(|k| {
            let idx = schema.index_of(&k.column).ok()?;
            let dt = schema.field(idx).data_type().clone();
            let opts = SortOptions {
                descending: k.descending,
                nulls_first: !k.nulls_last,
            };
            Some(SortField::new_with_options(dt, opts))
        })
        .collect();
    RowConverter::new(fields).map_err(EngineError::Arrow)
}

fn extract_key_cols(
    batch: &RecordBatch,
    sort_keys: &[SortKey],
) -> Result<Vec<ArrayRef>, EngineError> {
    sort_keys
        .iter()
        .map(|k| {
            let idx = batch
                .schema()
                .index_of(&k.column)
                .map_err(|_| EngineError::Query(format!("sort column not found: {}", k.column)))?;
            Ok(batch.column(idx).clone())
        })
        .collect()
}

fn emit_pending(
    pending: &[(usize, usize)],
    batches: &[Option<RecordBatch>],
    n_cols: usize,
    schema: &SchemaRef,
) -> Result<RecordBatch, EngineError> {
    if pending.is_empty() {
        return Ok(RecordBatch::new_empty(schema.clone()));
    }
    let dummy_batches: Vec<RecordBatch> = batches
        .iter()
        .map(|b| {
            b.clone()
                .unwrap_or_else(|| RecordBatch::new_empty(schema.clone()))
        })
        .collect();

    let mut out_cols: Vec<ArrayRef> = Vec::with_capacity(n_cols);
    for c in 0..n_cols {
        let arrs: Vec<&dyn Array> = dummy_batches.iter().map(|b| b.column(c).as_ref()).collect();
        let col = interleave(&arrs, pending)?;
        out_cols.push(col);
    }
    Ok(RecordBatch::try_new(schema.clone(), out_cols)?)
}

fn read_run_slice(
    run: &SpilledRun,
    row_offset: u64,
    row_limit: u64,
) -> Result<Vec<RecordBatch>, EngineError> {
    if row_limit == 0 {
        return Ok(vec![]);
    }
    let file = File::open(&run.path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder
        .with_offset(row_offset as usize)
        .with_limit(row_limit as usize)
        .with_batch_size(4096)
        .build()
        .map_err(EngineError::Parquet)?;
    reader
        .collect::<Result<Vec<_>, _>>()
        .map_err(EngineError::Arrow)
}

fn dedup_sorted_batches(
    batches: Vec<RecordBatch>,
    dedup_columns: &[String],
    schema: &SchemaRef,
) -> Result<Vec<RecordBatch>, EngineError> {
    if batches.is_empty() {
        return Ok(batches);
    }
    let combined = concat_batches(schema, &batches)?;
    let n = combined.num_rows();
    if n == 0 {
        return Ok(vec![combined]);
    }

    let key_indices: Vec<usize> = if dedup_columns.is_empty() {
        (0..combined.num_columns()).collect()
    } else {
        dedup_columns
            .iter()
            .filter_map(|name| combined.schema().index_of(name).ok())
            .collect()
    };

    let mut keep = Vec::with_capacity(n);
    keep.push(true);

    for i in 1..n {
        let dup = key_indices.iter().all(|&ki| {
            let col = combined.column(ki);
            let a_null = col.is_null(i);
            let b_null = col.is_null(i - 1);
            if a_null != b_null {
                return false;
            }
            if a_null {
                return true;
            }
            match cast(col.as_ref(), &ArrowDataType::Utf8) {
                Ok(str_col) => {
                    if let Some(s) = str_col.as_any().downcast_ref::<StringArray>() {
                        s.value(i) == s.value(i - 1)
                    } else {
                        false
                    }
                }
                Err(_) => false,
            }
        });
        keep.push(!dup);
    }

    let mask = BooleanArray::from(keep);
    Ok(vec![filter_record_batch(&combined, &mask)?])
}

fn slice_result(
    batches: Vec<RecordBatch>,
    row_offset: usize,
    row_limit: usize,
) -> Vec<RecordBatch> {
    let mut result = Vec::new();
    let mut skipped = 0usize;
    let mut collected = 0usize;
    for batch in batches {
        if collected >= row_limit {
            break;
        }
        let n = batch.num_rows();
        if skipped + n <= row_offset {
            skipped += n;
            continue;
        }
        let start = row_offset.saturating_sub(skipped);
        let available = n - start;
        let take = available.min(row_limit - collected);
        result.push(batch.slice(start, take));
        skipped += n;
        collected += take;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spill::SpillWriter;
    use crate::test_helpers::int_string_batch;
    use arrow::array::Int64Array;
    use tempfile::TempDir;
    use tv_core::SortKey;

    fn ascending_id_key() -> Vec<SortKey> {
        vec![SortKey {
            column: "id".to_string(),
            descending: false,
            nulls_last: true,
        }]
    }

    fn rows_from_batches(batches: &[RecordBatch]) -> Vec<i64> {
        batches
            .iter()
            .flat_map(|b| {
                b.column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values()
                    .to_vec()
            })
            .collect()
    }

    #[test]
    fn sort_to_runs_small_data() {
        let tmp = TempDir::new().unwrap();
        let batch = int_string_batch(50);
        let schema = batch.schema();
        let sorter = ExternalSorter::new(ascending_id_key(), schema.clone());
        let mut writer = SpillWriter::new(tmp.path().to_path_buf(), schema);
        let result = sorter
            .sort_to_runs(vec![Ok(batch)].into_iter(), &mut writer)
            .unwrap();
        assert_eq!(result.total_rows, 50);
        let batches = sorter.merge_all(&result.runs).unwrap();
        let ids = rows_from_batches(&batches);
        assert_eq!(ids.len(), 50);
        assert!(ids.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn sort_to_runs_large_data() {
        let tmp = TempDir::new().unwrap();
        let batch = int_string_batch(1000);
        let schema = batch.schema();
        let sorter = ExternalSorter::new(ascending_id_key(), schema.clone());
        let mut writer = SpillWriter::new(tmp.path().to_path_buf(), schema);
        let result = sorter
            .sort_to_runs(vec![Ok(batch)].into_iter(), &mut writer)
            .unwrap();
        assert_eq!(result.total_rows, 1000);
        let batches = sorter.merge_all(&result.runs).unwrap();
        let ids = rows_from_batches(&batches);
        assert_eq!(ids.len(), 1000);
        assert!(ids.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn merge_tile_offset_limit() {
        let tmp = TempDir::new().unwrap();
        let batch = int_string_batch(100);
        let schema = batch.schema();
        let sorter = ExternalSorter::new(ascending_id_key(), schema.clone());
        let mut writer = SpillWriter::new(tmp.path().to_path_buf(), schema);
        let result = sorter
            .sort_to_runs(vec![Ok(batch)].into_iter(), &mut writer)
            .unwrap();
        let tile = sorter
            .merge_tile(&result.runs, &result.cumulative_rows, 10, 20)
            .unwrap();
        let ids = rows_from_batches(&tile);
        assert_eq!(ids.len(), 20);
        assert_eq!(ids[0], 10);
        assert_eq!(ids[19], 29);
    }

    #[test]
    fn merge_all_preserves_sort_order() {
        let tmp = TempDir::new().unwrap();
        let batch1 = int_string_batch(60);
        let batch2 = int_string_batch(40);
        let schema = batch1.schema();
        let sorter = ExternalSorter::new(ascending_id_key(), schema.clone());
        let mut writer = SpillWriter::new(tmp.path().to_path_buf(), schema);
        let result = sorter
            .sort_to_runs(vec![Ok(batch1), Ok(batch2)].into_iter(), &mut writer)
            .unwrap();
        let batches = sorter.merge_all(&result.runs).unwrap();
        let ids = rows_from_batches(&batches);
        assert!(ids.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn cascade_merge_beyond_fanout() {
        let tmp = TempDir::new().unwrap();
        let schema = int_string_batch(1).schema();
        let sorter = ExternalSorter::new(ascending_id_key(), schema.clone());
        let mut writer = SpillWriter::new(tmp.path().to_path_buf(), schema.clone());

        let input_batches: Vec<Result<RecordBatch, EngineError>> =
            (0..10).map(|_| Ok(int_string_batch(10))).collect();
        let result = sorter
            .sort_to_runs(input_batches.into_iter(), &mut writer)
            .unwrap();
        assert_eq!(result.total_rows, 100);
        let batches = sorter.merge_all(&result.runs).unwrap();
        let ids = rows_from_batches(&batches);
        assert_eq!(ids.len(), 100);
        assert!(ids.windows(2).all(|w| w[0] <= w[1]));
    }
}
