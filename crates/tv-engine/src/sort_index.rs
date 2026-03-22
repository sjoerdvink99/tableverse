use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, UInt64Array};
use arrow::compute::take;
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::{ParquetRecordBatchReaderBuilder, RowSelection, RowSelector};
use parquet::arrow::ProjectionMask;
use tv_core::SortKey;

use crate::error::EngineError;
use crate::executor::apply_sort;

const MAGIC: u32 = 0x54564958;
const VERSION: u16 = 1;
const HEADER_SIZE: u64 = 24;

pub fn build(
    source_path: &str,
    sort_keys: &[SortKey],
    index_path: &Path,
) -> Result<(), EngineError> {
    let (schema, _) = crate::reader::parquet_schema_and_rows(source_path)?;

    let key_col_indices: Vec<usize> = sort_keys
        .iter()
        .filter_map(|k| schema.index_of(&k.column).ok())
        .collect();

    let key_batches = crate::reader::read_parquet_full(source_path, Some(&key_col_indices))?;

    let mut augmented: Vec<RecordBatch> = Vec::new();
    let mut global_row = 0u64;

    for batch in key_batches {
        let n = batch.num_rows();
        let row_ids: Arc<dyn Array> = Arc::new(UInt64Array::from_iter_values(
            global_row..global_row + n as u64,
        ));

        let mut fields: Vec<Arc<Field>> = batch.schema().fields().to_vec();
        fields.push(Arc::new(Field::new(
            "__row_id__",
            ArrowDataType::UInt64,
            false,
        )));
        let aug_schema = Arc::new(Schema::new(fields));

        let mut cols: Vec<ArrayRef> = batch.columns().to_vec();
        cols.push(row_ids);

        augmented.push(RecordBatch::try_new(aug_schema, cols)?);
        global_row += n as u64;
    }

    let total_rows = global_row;

    let mut sort_with_id = sort_keys.to_vec();
    sort_with_id.push(SortKey {
        column: "__row_id__".to_string(),
        descending: false,
        nulls_last: true,
    });

    let sorted = apply_sort(augmented, &sort_with_id)?;

    let row_id_col_name = "__row_id__";
    let mut sorted_row_ids: Vec<u64> = Vec::with_capacity(total_rows as usize);

    for batch in &sorted {
        let col_idx = batch
            .schema()
            .index_of(row_id_col_name)
            .map_err(|_| EngineError::Query("row_id column missing after sort".into()))?;
        let col = batch
            .column(col_idx)
            .as_any()
            .downcast_ref::<UInt64Array>()
            .ok_or_else(|| EngineError::Query("row_id column wrong type".into()))?;
        sorted_row_ids.extend(col.values().iter().copied());
    }

    let mut file = File::create(index_path)?;
    file.write_all(&MAGIC.to_le_bytes())?;
    file.write_all(&VERSION.to_le_bytes())?;
    file.write_all(&0u16.to_le_bytes())?;
    file.write_all(&total_rows.to_le_bytes())?;
    file.write_all(&sort_key_fingerprint(sort_keys).to_le_bytes())?;

    for id in &sorted_row_ids {
        file.write_all(&id.to_le_bytes())?;
    }

    file.flush()?;
    Ok(())
}

pub fn tile_lookup(
    index_path: &Path,
    row_offset: usize,
    limit: usize,
) -> Result<Vec<u64>, EngineError> {
    let mut file = File::open(index_path)?;

    let mut header = [0u8; HEADER_SIZE as usize];
    file.read_exact(&mut header)?;

    let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
    if magic != MAGIC {
        return Err(EngineError::Query("invalid sort index: bad magic".into()));
    }

    let row_count = u64::from_le_bytes(header[8..16].try_into().unwrap());

    let start = row_offset;
    let end = (start + limit).min(row_count as usize);
    let count = end.saturating_sub(start);

    if count == 0 {
        return Ok(vec![]);
    }

    let byte_offset = HEADER_SIZE + start as u64 * 8;
    file.seek(SeekFrom::Start(byte_offset))?;

    let mut buf = vec![0u8; count * 8];
    file.read_exact(&mut buf)?;

    let row_ids: Vec<u64> = buf
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
        .collect();

    Ok(row_ids)
}

pub fn row_count(index_path: &Path) -> Result<u64, EngineError> {
    let mut file = File::open(index_path)?;
    let mut header = [0u8; HEADER_SIZE as usize];
    file.read_exact(&mut header)?;
    let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
    if magic != MAGIC {
        return Err(EngineError::Query("invalid sort index: bad magic".into()));
    }
    Ok(u64::from_le_bytes(header[8..16].try_into().unwrap()))
}

pub fn read_rows_by_ids(
    source_path: &str,
    sorted_row_ids: &[u64],
    col_indices: &[usize],
) -> Result<Vec<RecordBatch>, EngineError> {
    if sorted_row_ids.is_empty() {
        return Ok(vec![]);
    }

    let mut indexed: Vec<(usize, u64)> = sorted_row_ids.iter().copied().enumerate().collect();
    indexed.sort_by_key(|&(_, id)| id);

    let mut selectors: Vec<RowSelector> = Vec::new();
    let mut file_pos = 0u64;

    for &(_, row_id) in &indexed {
        if row_id > file_pos {
            selectors.push(RowSelector::skip((row_id - file_pos) as usize));
        }
        selectors.push(RowSelector::select(1));
        file_pos = row_id + 1;
    }

    let row_selection = RowSelection::from(selectors);

    let file = File::open(source_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let mask = ProjectionMask::roots(builder.parquet_schema(), col_indices.iter().copied());

    let reader = builder
        .with_projection(mask)
        .with_row_selection(row_selection)
        .with_batch_size(sorted_row_ids.len().max(1))
        .build()
        .map_err(EngineError::Parquet)?;

    let file_order_batches: Vec<RecordBatch> = reader
        .collect::<Result<Vec<_>, _>>()
        .map_err(EngineError::Arrow)?;

    if file_order_batches.is_empty() {
        return Ok(vec![]);
    }

    let schema = file_order_batches[0].schema();
    let combined = if file_order_batches.len() == 1 {
        file_order_batches.into_iter().next().unwrap()
    } else {
        arrow::compute::concat_batches(&schema, &file_order_batches)?
    };

    let n = sorted_row_ids.len();
    let mut perm = vec![0u32; n];
    for (file_read_idx, &(sorted_pos, _)) in indexed.iter().enumerate() {
        perm[sorted_pos] = file_read_idx as u32;
    }

    let indices = arrow::array::UInt32Array::from(perm);
    let cols: Vec<ArrayRef> = (0..combined.num_columns())
        .map(|i| take(combined.column(i).as_ref(), &indices, None))
        .collect::<Result<_, _>>()?;

    Ok(vec![RecordBatch::try_new(schema, cols)?])
}

fn sort_key_fingerprint(keys: &[SortKey]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for k in keys {
        k.column.hash(&mut hasher);
        k.descending.hash(&mut hasher);
        k.nulls_last.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{int_string_batch, write_test_parquet};
    use tempfile::TempDir;
    use tv_core::SortKey;

    fn make_dir() -> TempDir {
        tempfile::TempDir::new().unwrap()
    }

    #[test]
    fn build_and_tile_lookup_roundtrip() {
        let dir = make_dir();
        let batch = int_string_batch(50);
        let parquet_path = write_test_parquet(&dir, "data.parquet", &[batch]);
        let index_path = dir.path().join("data.tvi");

        let sort_keys = vec![SortKey {
            column: "id".to_string(),
            descending: false,
            nulls_last: true,
        }];

        build(parquet_path.to_str().unwrap(), &sort_keys, &index_path).unwrap();

        let row_ids = tile_lookup(&index_path, 0, 10).unwrap();
        assert_eq!(row_ids.len(), 10);
        assert!(row_ids.iter().all(|&id| id < 50));
    }

    #[test]
    fn tile_lookup_offset_limit() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let parquet_path = write_test_parquet(&dir, "data.parquet", &[batch]);
        let index_path = dir.path().join("data.tvi");

        let sort_keys = vec![SortKey {
            column: "id".to_string(),
            descending: false,
            nulls_last: true,
        }];

        build(parquet_path.to_str().unwrap(), &sort_keys, &index_path).unwrap();

        let first_ten = tile_lookup(&index_path, 0, 10).unwrap();
        assert_eq!(first_ten.len(), 10);

        let mid_ten = tile_lookup(&index_path, 50, 10).unwrap();
        assert_eq!(mid_ten.len(), 10);

        let last_partial = tile_lookup(&index_path, 95, 10).unwrap();
        assert_eq!(last_partial.len(), 5);
    }

    #[test]
    fn tile_lookup_beyond_end() {
        let dir = make_dir();
        let batch = int_string_batch(20);
        let parquet_path = write_test_parquet(&dir, "data.parquet", &[batch]);
        let index_path = dir.path().join("data.tvi");

        let sort_keys = vec![SortKey {
            column: "id".to_string(),
            descending: false,
            nulls_last: true,
        }];

        build(parquet_path.to_str().unwrap(), &sort_keys, &index_path).unwrap();

        let result = tile_lookup(&index_path, 20, 10).unwrap();
        assert!(result.is_empty());

        let result2 = tile_lookup(&index_path, 100, 10).unwrap();
        assert!(result2.is_empty());
    }

    #[test]
    fn row_count_matches_source() {
        let dir = make_dir();
        let batch = int_string_batch(77);
        let parquet_path = write_test_parquet(&dir, "data.parquet", &[batch]);
        let index_path = dir.path().join("data.tvi");

        let sort_keys = vec![SortKey {
            column: "score".to_string(),
            descending: true,
            nulls_last: true,
        }];

        build(parquet_path.to_str().unwrap(), &sort_keys, &index_path).unwrap();

        let n = row_count(&index_path).unwrap();
        assert_eq!(n, 77);
    }
}
