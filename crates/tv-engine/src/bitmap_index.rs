use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, Int64Array, StringArray};
use arrow::compute::cast;
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use tv_core::Literal;

use crate::error::EngineError;

const MAGIC: u32 = 0x54564446;
const VERSION: u16 = 1;
const MAX_CARDINALITY: usize = 65_536;
const MIN_ROWS_FOR_INDEX: u64 = 1_000_000;

pub struct BitmapIndex {
    dictionary: Vec<String>,
    bitmaps: Vec<RoaringLike>,
    n_rows: u64,
}

struct RoaringLike {
    runs: Vec<(u32, u32)>,
}

impl RoaringLike {
    fn new() -> Self {
        Self { runs: Vec::new() }
    }

    fn insert(&mut self, row: u32) {
        self.runs.push((row, row));
        self.runs.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(self.runs.len());
        for (start, end) in &self.runs {
            if let Some(last) = merged.last_mut() {
                if *start <= last.1 + 1 {
                    last.1 = last.1.max(*end);
                    continue;
                }
            }
            merged.push((*start, *end));
        }
        self.runs = merged;
    }

    fn count(&self) -> u64 {
        self.runs.iter().map(|(s, e)| (e - s + 1) as u64).sum()
    }

    fn iter_rows(&self) -> impl Iterator<Item = u32> + '_ {
        self.runs.iter().flat_map(|&(s, e)| s..=e)
    }

    fn serialize(&self, w: &mut dyn Write) -> std::io::Result<()> {
        w.write_all(&(self.runs.len() as u32).to_le_bytes())?;
        for &(s, e) in &self.runs {
            w.write_all(&s.to_le_bytes())?;
            w.write_all(&e.to_le_bytes())?;
        }
        Ok(())
    }

    fn deserialize(r: &mut dyn Read) -> std::io::Result<Self> {
        let mut buf4 = [0u8; 4];
        r.read_exact(&mut buf4)?;
        let n_runs = u32::from_le_bytes(buf4) as usize;
        let mut runs = Vec::with_capacity(n_runs);
        for _ in 0..n_runs {
            r.read_exact(&mut buf4)?;
            let start = u32::from_le_bytes(buf4);
            r.read_exact(&mut buf4)?;
            let end = u32::from_le_bytes(buf4);
            runs.push((start, end));
        }
        Ok(Self { runs })
    }
}

impl BitmapIndex {
    pub fn count_for_value(&self, value: &Literal) -> Option<u64> {
        let key = literal_to_str(value)?;
        let idx = self.dictionary.iter().position(|v| v == key)?;
        Some(self.bitmaps[idx].count())
    }

    pub fn row_ids_for_value(&self, value: &Literal) -> Option<Vec<u64>> {
        let key = literal_to_str(value)?;
        let idx = self.dictionary.iter().position(|v| v == key)?;
        Some(self.bitmaps[idx].iter_rows().map(|r| r as u64).collect())
    }

    pub fn group_by_count(&self, agg_alias: &str) -> Option<RecordBatch> {
        let keys: Vec<Option<&str>> = self.dictionary.iter().map(|s| Some(s.as_str())).collect();
        let counts: Vec<i64> = self.bitmaps.iter().map(|b| b.count() as i64).collect();

        let key_col: ArrayRef = Arc::new(StringArray::from(keys));
        let count_col: ArrayRef = Arc::new(Int64Array::from(counts));

        let schema = Arc::new(Schema::new(vec![
            Field::new("__key__", ArrowDataType::Utf8, true),
            Field::new(agg_alias, ArrowDataType::Int64, true),
        ]));

        RecordBatch::try_new(schema, vec![key_col, count_col]).ok()
    }

    pub fn n_rows(&self) -> u64 {
        self.n_rows
    }
}

pub fn should_build(n_rows: u64, distinct_count: u64) -> bool {
    n_rows >= MIN_ROWS_FOR_INDEX && distinct_count <= MAX_CARDINALITY as u64
}

pub fn build(
    source_path: &str,
    col_idx: usize,
    _schema: &SchemaRef,
    index_path: &Path,
) -> Result<(), EngineError> {
    let batches = crate::reader::read_parquet_full(source_path, Some(&[col_idx]))?;

    let mut dict: HashMap<String, RoaringLike> = HashMap::new();
    let mut global_row = 0u32;

    for batch in &batches {
        let col = batch.column(0);
        let as_str = cast(col.as_ref(), &ArrowDataType::Utf8)?;
        if let Some(str_arr) = as_str.as_any().downcast_ref::<StringArray>() {
            for i in 0..str_arr.len() {
                let key = if str_arr.is_null(i) {
                    "__null__".to_string()
                } else {
                    str_arr.value(i).to_string()
                };
                dict.entry(key)
                    .or_insert_with(RoaringLike::new)
                    .insert(global_row);
                global_row += 1;
            }
        }
    }

    if dict.len() > MAX_CARDINALITY {
        return Err(EngineError::Query(format!(
            "bitmap index: cardinality {} exceeds limit {}",
            dict.len(),
            MAX_CARDINALITY
        )));
    }

    let mut sorted_pairs: Vec<(String, RoaringLike)> = dict.into_iter().collect();
    sorted_pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let n_values = sorted_pairs.len() as u32;
    let n_rows = global_row as u64;

    let out_file = File::create(index_path)?;
    let mut writer = BufWriter::new(out_file);

    writer.write_all(&MAGIC.to_le_bytes())?;
    writer.write_all(&VERSION.to_le_bytes())?;
    writer.write_all(&n_values.to_le_bytes())?;
    writer.write_all(&n_rows.to_le_bytes())?;

    for (key, bitmap) in &sorted_pairs {
        let key_bytes = key.as_bytes();
        writer.write_all(&(key_bytes.len() as u32).to_le_bytes())?;
        writer.write_all(key_bytes)?;
        bitmap.serialize(&mut writer)?;
    }

    writer.flush()?;
    Ok(())
}

pub fn load(index_path: &Path) -> Result<BitmapIndex, EngineError> {
    let file = File::open(index_path)?;
    let mut reader = BufReader::new(file);

    let mut header = [0u8; 18];
    reader.read_exact(&mut header)?;

    let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
    if magic != MAGIC {
        return Err(EngineError::Query("invalid bitmap index: bad magic".into()));
    }

    let n_values = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
    let n_rows = u64::from_le_bytes(header[10..18].try_into().unwrap());

    let mut dictionary = Vec::with_capacity(n_values);
    let mut bitmaps = Vec::with_capacity(n_values);

    for _ in 0..n_values {
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;
        let key_len = u32::from_le_bytes(len_buf) as usize;
        let mut key_bytes = vec![0u8; key_len];
        reader.read_exact(&mut key_bytes)?;
        let key = String::from_utf8(key_bytes)
            .map_err(|e| EngineError::Query(format!("bitmap index: invalid utf8: {e}")))?;
        let bitmap = RoaringLike::deserialize(&mut reader)?;
        dictionary.push(key);
        bitmaps.push(bitmap);
    }

    Ok(BitmapIndex {
        dictionary,
        bitmaps,
        n_rows,
    })
}

pub fn fetch_rows_for_value(
    source_path: &str,
    schema: &SchemaRef,
    row_ids: &[u64],
) -> Result<Vec<RecordBatch>, EngineError> {
    let col_indices: Vec<usize> = (0..schema.fields().len()).collect();
    crate::sort_index::read_rows_by_ids(source_path, row_ids, &col_indices)
}

pub fn execute_group_by_count(
    bitmap_index: &BitmapIndex,
    key_col_name: &str,
    count_alias: &str,
) -> Result<RecordBatch, EngineError> {
    bitmap_index
        .group_by_count(count_alias)
        .ok_or_else(|| EngineError::Query("bitmap group_by failed".into()))
        .and_then(|b| {
            let schema = Arc::new(Schema::new(vec![
                Field::new(key_col_name, ArrowDataType::Utf8, true),
                Field::new(count_alias, ArrowDataType::Int64, true),
            ]));
            let key_col = b.column(0).clone();
            let count_col = b.column(1).clone();
            RecordBatch::try_new(schema, vec![key_col, count_col]).map_err(EngineError::Arrow)
        })
}

fn literal_to_str(lit: &Literal) -> Option<&str> {
    match lit {
        Literal::Text(s) => Some(s.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{int_string_batch, write_multi_rg_parquet};
    use tempfile::TempDir;
    use tv_core::Literal;

    fn make_dir() -> TempDir {
        tempfile::TempDir::new().unwrap()
    }

    fn write_parquet_with_name_col(dir: &TempDir, rows: usize) -> std::path::PathBuf {
        let batch = int_string_batch(rows);
        write_multi_rg_parquet(dir, "data.parquet", &[batch], rows + 1)
    }

    #[test]
    fn build_and_load_roundtrip() {
        let dir = make_dir();
        let parquet_path = write_parquet_with_name_col(&dir, 10);
        let index_path = dir.path().join("data.tvd");
        let schema = std::sync::Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("id", arrow::datatypes::DataType::Int64, false),
            arrow::datatypes::Field::new("name", arrow::datatypes::DataType::Utf8, false),
        ]));

        build(parquet_path.to_str().unwrap(), 1, &schema, &index_path).unwrap();

        let loaded = load(&index_path).unwrap();
        assert_eq!(loaded.n_rows(), 10);
        assert!(!loaded.dictionary.is_empty());
    }

    #[test]
    fn count_for_value_correct() {
        let dir = make_dir();
        let parquet_path = write_parquet_with_name_col(&dir, 20);
        let index_path = dir.path().join("data.tvd");
        let schema = std::sync::Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("id", arrow::datatypes::DataType::Int64, false),
            arrow::datatypes::Field::new("name", arrow::datatypes::DataType::Utf8, false),
        ]));

        build(parquet_path.to_str().unwrap(), 1, &schema, &index_path).unwrap();
        let loaded = load(&index_path).unwrap();

        let count = loaded.count_for_value(&Literal::Text("item_0".to_string()));
        assert_eq!(count, Some(1));
    }

    #[test]
    fn row_ids_for_value_correct() {
        let dir = make_dir();
        let parquet_path = write_parquet_with_name_col(&dir, 10);
        let index_path = dir.path().join("data.tvd");
        let schema = std::sync::Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("id", arrow::datatypes::DataType::Int64, false),
            arrow::datatypes::Field::new("name", arrow::datatypes::DataType::Utf8, false),
        ]));

        build(parquet_path.to_str().unwrap(), 1, &schema, &index_path).unwrap();
        let loaded = load(&index_path).unwrap();

        let row_ids = loaded.row_ids_for_value(&Literal::Text("item_3".to_string()));
        assert!(row_ids.is_some());
        let ids = row_ids.unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], 3);
    }

    #[test]
    fn group_by_count_all_values() {
        let dir = make_dir();
        let parquet_path = write_parquet_with_name_col(&dir, 10);
        let index_path = dir.path().join("data.tvd");
        let schema = std::sync::Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("id", arrow::datatypes::DataType::Int64, false),
            arrow::datatypes::Field::new("name", arrow::datatypes::DataType::Utf8, false),
        ]));

        build(parquet_path.to_str().unwrap(), 1, &schema, &index_path).unwrap();
        let loaded = load(&index_path).unwrap();

        let batch = loaded.group_by_count("cnt");
        assert!(batch.is_some());
        let batch = batch.unwrap();
        assert_eq!(batch.num_columns(), 2);
        assert_eq!(batch.num_rows(), 10);

        let count_col = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .unwrap();
        let total: i64 = count_col.values().iter().sum();
        assert_eq!(total, 10);
    }

    #[test]
    fn exceeds_max_cardinality() {
        let dir = make_dir();
        let n = MAX_CARDINALITY + 1;
        let schema = std::sync::Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("tag", arrow::datatypes::DataType::Utf8, false),
        ]));
        let values: Vec<String> = (0..n).map(|i| format!("val_{i}")).collect();
        let str_refs: Vec<&str> = values.iter().map(|s| s.as_str()).collect();
        let batch = arrow::record_batch::RecordBatch::try_new(
            schema.clone(),
            vec![std::sync::Arc::new(arrow::array::StringArray::from(
                str_refs,
            ))],
        )
        .unwrap();

        let parquet_path = write_multi_rg_parquet(&dir, "big.parquet", &[batch], n + 1);
        let index_path = dir.path().join("big.tvd");

        let result = build(parquet_path.to_str().unwrap(), 0, &schema, &index_path);
        assert!(result.is_err());
    }
}
