use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use arrow::array::{Array, StringArray};
use arrow::compute::cast;
use arrow::datatypes::{DataType as ArrowDataType, SchemaRef};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ProjectionMask;
use tv_core::{Literal, Predicate};

use crate::error::EngineError;

const MAGIC: u32 = 0x54564246;
const VERSION: u16 = 1;
const BITS_PER_ELEMENT: f64 = 14.4;
const NUM_HASH_FUNCS: u32 = 2;

pub struct BloomFilter {
    bits: Vec<u8>,
    n_bits: u32,
}

impl BloomFilter {
    fn new(n_elements: u32) -> Self {
        let n_bits = ((n_elements as f64 * BITS_PER_ELEMENT) as u32).max(64);
        let n_bytes = (n_bits as usize).div_ceil(8);
        Self {
            bits: vec![0u8; n_bytes],
            n_bits,
        }
    }

    fn from_bits(bits: Vec<u8>, n_bits: u32) -> Self {
        Self { bits, n_bits }
    }

    fn set_bit(&mut self, pos: u32) {
        let idx = (pos % self.n_bits) as usize;
        self.bits[idx / 8] |= 1 << (idx % 8);
    }

    fn get_bit(&self, pos: u32) -> bool {
        let idx = (pos % self.n_bits) as usize;
        (self.bits[idx / 8] >> (idx % 8)) & 1 == 1
    }

    fn insert_hash(&mut self, hash: u64) {
        let h1 = hash as u32;
        let h2 = (hash >> 32) as u32;
        for i in 0..NUM_HASH_FUNCS {
            self.set_bit(h1.wrapping_add(i.wrapping_mul(h2)));
        }
    }

    pub fn might_contain_hash(&self, hash: u64) -> bool {
        let h1 = hash as u32;
        let h2 = (hash >> 32) as u32;
        for i in 0..NUM_HASH_FUNCS {
            if !self.get_bit(h1.wrapping_add(i.wrapping_mul(h2))) {
                return false;
            }
        }
        true
    }
}

pub struct BloomIndex {
    filters: HashMap<(u32, u32), BloomFilter>,
}

impl BloomIndex {
    pub fn might_skip_row_group(&self, rg_idx: usize, col_idx: usize, value: &Literal) -> bool {
        let key = (rg_idx as u32, col_idx as u32);
        match self.filters.get(&key) {
            None => false,
            Some(filter) => {
                let hash = literal_hash(value);
                !filter.might_contain_hash(hash)
            }
        }
    }

    pub fn might_skip_for_any(&self, rg_idx: usize, col_idx: usize, values: &[Literal]) -> bool {
        let key = (rg_idx as u32, col_idx as u32);
        match self.filters.get(&key) {
            None => false,
            Some(filter) => values
                .iter()
                .all(|v| !filter.might_contain_hash(literal_hash(v))),
        }
    }
}

pub fn build(source_path: &str, schema: &SchemaRef, index_path: &Path) -> Result<(), EngineError> {
    let file = File::open(source_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let metadata = builder.metadata().clone();
    let n_row_groups = metadata.num_row_groups() as u32;
    let n_cols = schema.fields().len() as u32;

    let mut col_row_counts: Vec<u64> = vec![0u64; n_cols as usize];
    let mut filters: HashMap<(u32, u32), BloomFilter> = HashMap::new();

    for rg_idx in 0..n_row_groups as usize {
        let rg = metadata.row_group(rg_idx);
        let rg_rows = rg.num_rows() as u64;

        for count in col_row_counts.iter_mut().take(n_cols as usize) {
            *count += rg_rows;
        }
    }

    for rg_idx in 0..n_row_groups as usize {
        let rg = metadata.row_group(rg_idx);
        let rg_rows = rg.num_rows() as u32;

        for col_idx in 0..n_cols as usize {
            let mask = ProjectionMask::roots(builder.parquet_schema(), [col_idx]);

            let file2 = File::open(source_path)?;
            let b2 = ParquetRecordBatchReaderBuilder::try_new(file2)?;
            let reader = b2
                .with_projection(mask)
                .with_row_groups(vec![rg_idx])
                .build()
                .map_err(EngineError::Parquet)?;

            let mut bloom = BloomFilter::new(rg_rows);

            for batch in reader {
                let batch = batch.map_err(EngineError::Arrow)?;
                let col = batch.column(0);
                let as_str = cast(col.as_ref(), &ArrowDataType::Utf8)?;
                if let Some(str_arr) = as_str.as_any().downcast_ref::<StringArray>() {
                    for i in 0..str_arr.len() {
                        if str_arr.is_null(i) {
                            bloom.insert_hash(literal_hash(&Literal::Null));
                        } else {
                            bloom.insert_hash(str_hash(str_arr.value(i)));
                        }
                    }
                }
            }

            filters.insert((rg_idx as u32, col_idx as u32), bloom);
        }
    }

    let out_file = File::create(index_path)?;
    let mut writer = BufWriter::new(out_file);

    writer.write_all(&MAGIC.to_le_bytes())?;
    writer.write_all(&VERSION.to_le_bytes())?;
    writer.write_all(&n_row_groups.to_le_bytes())?;
    writer.write_all(&n_cols.to_le_bytes())?;

    for rg_idx in 0..n_row_groups {
        for col_idx in 0..n_cols {
            let key = (rg_idx, col_idx);
            if let Some(bloom) = filters.get(&key) {
                writer.write_all(&col_idx.to_le_bytes())?;
                writer.write_all(&rg_idx.to_le_bytes())?;
                writer.write_all(&bloom.n_bits.to_le_bytes())?;
                let n_bytes = (bloom.n_bits as usize).div_ceil(8);
                writer.write_all(&(n_bytes as u32).to_le_bytes())?;
                writer.write_all(&bloom.bits[..n_bytes])?;
            }
        }
    }

    writer.flush()?;
    Ok(())
}

pub fn load(index_path: &Path) -> Result<BloomIndex, EngineError> {
    let file = File::open(index_path)?;
    let mut reader = BufReader::new(file);

    let mut header = [0u8; 16];
    reader.read_exact(&mut header)?;

    let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
    if magic != MAGIC {
        return Err(EngineError::Query("invalid bloom index: bad magic".into()));
    }

    let n_row_groups = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let n_cols = u32::from_le_bytes(header[12..16].try_into().unwrap());

    let mut filters: HashMap<(u32, u32), BloomFilter> = HashMap::new();

    let total_entries = n_row_groups as usize * n_cols as usize;
    let mut entry_header = [0u8; 16];

    for _ in 0..total_entries {
        if reader.read_exact(&mut entry_header).is_err() {
            break;
        }
        let col_idx = u32::from_le_bytes(entry_header[0..4].try_into().unwrap());
        let rg_idx = u32::from_le_bytes(entry_header[4..8].try_into().unwrap());
        let n_bits = u32::from_le_bytes(entry_header[8..12].try_into().unwrap());
        let n_bytes = u32::from_le_bytes(entry_header[12..16].try_into().unwrap()) as usize;

        let mut bits = vec![0u8; n_bytes];
        reader.read_exact(&mut bits)?;

        filters.insert((rg_idx, col_idx), BloomFilter::from_bits(bits, n_bits));
    }

    Ok(BloomIndex { filters })
}

pub fn can_prune_row_group_with_bloom(
    bloom: &BloomIndex,
    rg_idx: usize,
    predicate: &Predicate,
    schema: &SchemaRef,
) -> bool {
    match predicate {
        Predicate::Eq { column, value } => {
            if let Ok(col_idx) = schema.index_of(column) {
                return bloom.might_skip_row_group(rg_idx, col_idx, value);
            }
            false
        }
        Predicate::In { column, values } => {
            if let Ok(col_idx) = schema.index_of(column) {
                return bloom.might_skip_for_any(rg_idx, col_idx, values);
            }
            false
        }
        Predicate::And { exprs } => exprs
            .iter()
            .any(|e| can_prune_row_group_with_bloom(bloom, rg_idx, e, schema)),
        _ => false,
    }
}

fn literal_hash(lit: &Literal) -> u64 {
    match lit {
        Literal::Null => mix64(0xDEAD_BEEF_DEAD_BEEFu64),
        Literal::Bool(b) => mix64(if *b { 1 } else { 0 }),
        Literal::Int(i) => mix64(*i as u64),
        Literal::Float(f) => mix64(f64::to_bits(*f)),
        Literal::Text(s) => str_hash(s),
    }
}

fn str_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    mix64(h)
}

fn mix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{int_string_batch, write_multi_rg_parquet};
    use tempfile::TempDir;

    fn make_dir() -> TempDir {
        tempfile::TempDir::new().unwrap()
    }

    #[test]
    fn bloom_filter_insert_and_query() {
        let mut bf = BloomFilter::new(100);
        let hash = 0xdeadbeef_cafebabe_u64;
        bf.insert_hash(hash);
        assert!(bf.might_contain_hash(hash));
    }

    #[test]
    fn bloom_filter_no_false_negatives() {
        let mut bf = BloomFilter::new(1000);
        let hashes: Vec<u64> = (0..500u64)
            .map(|i| i.wrapping_mul(0x9e3779b97f4a7c15))
            .collect();
        for &h in &hashes {
            bf.insert_hash(h);
        }
        for &h in &hashes {
            assert!(bf.might_contain_hash(h), "false negative for hash {h}");
        }
    }

    #[test]
    fn bloom_filter_false_positive_rate() {
        let n = 1000u32;
        let mut bf = BloomFilter::new(n);
        for i in 0..n {
            bf.insert_hash((i as u64).wrapping_mul(0x1234567890abcdef));
        }
        let mut false_positives = 0;
        let test_count = 10_000u32;
        for i in 0..test_count {
            let test_hash = (i as u64 + 1_000_000u64).wrapping_mul(0xfedcba9876543210u64);
            if bf.might_contain_hash(test_hash) {
                false_positives += 1;
            }
        }
        let fpr = false_positives as f64 / test_count as f64;
        assert!(fpr < 0.15, "false positive rate {fpr:.3} too high");
    }

    #[test]
    fn bloom_index_build_and_load_roundtrip() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let schema = batch.schema();
        let parquet_path = write_multi_rg_parquet(&dir, "data.parquet", &[batch], 25);
        let index_path = dir.path().join("data.tvb");

        build(parquet_path.to_str().unwrap(), &schema, &index_path).unwrap();
        assert!(index_path.exists());

        let bloom = load(&index_path).unwrap();
        let _ = bloom;
    }

    #[test]
    fn bloom_index_no_prune_eq_present() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let schema = batch.schema();
        let parquet_path = write_multi_rg_parquet(&dir, "data.parquet", &[batch], 25);
        let index_path = dir.path().join("data.tvb");
        build(parquet_path.to_str().unwrap(), &schema, &index_path).unwrap();
        let bloom = load(&index_path).unwrap();

        let pred = Predicate::Eq {
            column: "name".into(),
            value: Literal::Text("item_0".into()),
        };
        let should_prune = can_prune_row_group_with_bloom(&bloom, 0, &pred, &schema);
        assert!(!should_prune, "item_0 is present so should not prune rg 0");
    }

    #[test]
    fn bloom_index_cannot_prune_without_filter() {
        let dir = make_dir();
        let batch = int_string_batch(50);
        let schema = batch.schema();
        let parquet_path = write_multi_rg_parquet(&dir, "data.parquet", &[batch], 25);
        let index_path = dir.path().join("data.tvb");
        build(parquet_path.to_str().unwrap(), &schema, &index_path).unwrap();
        let bloom = load(&index_path).unwrap();

        let pred = Predicate::Eq {
            column: "name".into(),
            value: Literal::Null,
        };
        let should_prune = can_prune_row_group_with_bloom(&bloom, 0, &pred, &schema);
        assert!(!should_prune, "no nulls inserted so filter may prune, but null literal should not prune present-value filter");
        let _ = should_prune;
    }

    #[test]
    fn bloom_filter_absent_hash_not_present() {
        let mut bf = BloomFilter::new(1000);
        for i in 0u64..500 {
            bf.insert_hash(str_hash(&format!("item_{i}")));
        }
        let absent = str_hash("definitely_not_in_the_filter_xyzzy_42");
        assert!(
            !bf.might_contain_hash(absent),
            "unexpected false positive for highly specific absent string"
        );
    }

    #[test]
    fn can_prune_with_empty_bloom_returns_false() {
        let bloom = BloomIndex {
            filters: std::collections::HashMap::new(),
        };
        let schema = int_string_batch(10).schema();
        let pred = Predicate::Eq {
            column: "name".into(),
            value: Literal::Text("item_0".into()),
        };
        let result = can_prune_row_group_with_bloom(&bloom, 0, &pred, &schema);
        assert!(!result, "no filter entry => cannot prune");
    }

    #[test]
    fn bloom_index_prune_in_requires_all_absent() {
        let dir = make_dir();
        let batch = int_string_batch(100);
        let schema = batch.schema();
        let parquet_path = write_multi_rg_parquet(&dir, "data.parquet", &[batch], 25);
        let index_path = dir.path().join("data.tvb");
        build(parquet_path.to_str().unwrap(), &schema, &index_path).unwrap();
        let bloom = load(&index_path).unwrap();

        let pred_mixed = Predicate::In {
            column: "name".into(),
            values: vec![
                Literal::Text("item_0".into()),
                Literal::Text("xyz_absent_99999".into()),
            ],
        };
        let should_prune = can_prune_row_group_with_bloom(&bloom, 0, &pred_mixed, &schema);
        assert!(
            !should_prune,
            "item_0 is present so In predicate should not prune"
        );
    }
}
