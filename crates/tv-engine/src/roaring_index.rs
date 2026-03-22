use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};

use arrow::array::{Array, StringArray};
use arrow::compute::cast;
use arrow::datatypes::{DataType as ArrowDataType, SchemaRef};
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::arrow::ProjectionMask;
use tv_core::{Literal, Predicate};

use crate::error::EngineError;

const MAGIC: u32 = 0x54565246;
const VERSION: u16 = 1;
const MAX_CARDINALITY: usize = 10_000;

pub struct RoaringIndex {
    pub filters: HashMap<String, Vec<u64>>,
    n_row_groups: usize,
}

impl RoaringIndex {
    fn new(n_row_groups: usize) -> Self {
        Self {
            filters: HashMap::new(),
            n_row_groups,
        }
    }

    fn insert(&mut self, value: &str, rg_idx: usize) {
        let word = rg_idx / 64;
        let bit = rg_idx % 64;
        let entry = self.filters.entry(value.to_string()).or_default();
        while entry.len() <= word {
            entry.push(0);
        }
        entry[word] |= 1u64 << bit;
    }

    pub fn row_groups_for_eq(&self, value: &str, n_rg: usize) -> Option<Vec<usize>> {
        let bitmask = self.filters.get(value)?;
        let mut result = Vec::new();
        for (word_idx, &word) in bitmask.iter().enumerate() {
            let base = word_idx * 64;
            let mut w = word;
            while w != 0 {
                let bit = w.trailing_zeros() as usize;
                let rg = base + bit;
                if rg < n_rg {
                    result.push(rg);
                }
                w &= w - 1;
            }
        }
        Some(result)
    }

    pub fn row_groups_for_in(&self, values: &[&str], n_rg: usize) -> Vec<usize> {
        let n_words = n_rg.div_ceil(64);
        let mut union: Vec<u64> = vec![0u64; n_words];
        for &val in values {
            if let Some(bitmask) = self.filters.get(val) {
                for (i, &word) in bitmask.iter().enumerate() {
                    if i < union.len() {
                        union[i] |= word;
                    }
                }
            }
        }
        let mut result = Vec::new();
        for (word_idx, &word) in union.iter().enumerate() {
            let base = word_idx * 64;
            let mut w = word;
            while w != 0 {
                let bit = w.trailing_zeros() as usize;
                let rg = base + bit;
                if rg < n_rg {
                    result.push(rg);
                }
                w &= w - 1;
            }
        }
        result
    }
}

pub fn build(
    source_path: &str,
    col_idx: usize,
    schema: &SchemaRef,
) -> Result<Option<RoaringIndex>, EngineError> {
    let field = schema.field(col_idx);
    if !matches!(
        field.data_type(),
        ArrowDataType::Utf8 | ArrowDataType::LargeUtf8
    ) {
        return Ok(None);
    }

    let file = File::open(source_path)?;
    let metadata = ArrowReaderMetadata::load(&file, ArrowReaderOptions::new())
        .map_err(EngineError::Parquet)?;
    let n_rg = metadata.metadata().num_row_groups();
    let mut index = RoaringIndex::new(n_rg);

    for rg_idx in 0..n_rg {
        let file2 = File::open(source_path)?;
        let arm2 =
            ArrowReaderMetadata::try_new(metadata.metadata().clone(), ArrowReaderOptions::new())
                .map_err(EngineError::Parquet)?;
        let proj = ProjectionMask::roots(metadata.parquet_schema(), [col_idx]);
        let reader = ParquetRecordBatchReaderBuilder::new_with_metadata(file2, arm2)
            .with_projection(proj)
            .with_row_groups(vec![rg_idx])
            .build()
            .map_err(EngineError::Parquet)?;

        let mut distinct: std::collections::HashSet<String> = std::collections::HashSet::new();
        for batch_result in reader {
            let batch = batch_result.map_err(EngineError::Arrow)?;
            let col = batch.column(0);
            let utf8 = cast(col.as_ref(), &ArrowDataType::Utf8)?;
            if let Some(s) = utf8.as_any().downcast_ref::<StringArray>() {
                for i in 0..s.len() {
                    if !s.is_null(i) {
                        distinct.insert(s.value(i).to_string());
                        if index.filters.len() + distinct.len() > MAX_CARDINALITY {
                            return Ok(None);
                        }
                    }
                }
            }
        }

        for val in &distinct {
            index.insert(val, rg_idx);
        }

        if index.filters.len() > MAX_CARDINALITY {
            return Ok(None);
        }
    }

    Ok(Some(index))
}

pub fn save(index: &RoaringIndex, path: &str) -> Result<(), EngineError> {
    let file = File::create(path)?;
    let mut w = BufWriter::new(file);
    w.write_all(&MAGIC.to_le_bytes())?;
    w.write_all(&VERSION.to_le_bytes())?;
    let n_entries = index.filters.len() as u32;
    w.write_all(&n_entries.to_le_bytes())?;
    let n_rg = index.n_row_groups as u32;
    w.write_all(&n_rg.to_le_bytes())?;
    for (val, bitmask) in &index.filters {
        let vlen = val.len() as u32;
        w.write_all(&vlen.to_le_bytes())?;
        w.write_all(val.as_bytes())?;
        let wlen = bitmask.len() as u32;
        w.write_all(&wlen.to_le_bytes())?;
        for &word in bitmask {
            w.write_all(&word.to_le_bytes())?;
        }
    }
    w.flush()?;
    Ok(())
}

pub fn load(path: &str) -> Result<RoaringIndex, EngineError> {
    let file = File::open(path)?;
    let mut r = BufReader::new(file);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if u32::from_le_bytes(magic) != MAGIC {
        return Err(EngineError::Query("invalid roaring index magic".into()));
    }
    let mut ver = [0u8; 2];
    r.read_exact(&mut ver)?;
    let mut ne = [0u8; 4];
    r.read_exact(&mut ne)?;
    let n_entries = u32::from_le_bytes(ne) as usize;
    let mut nr = [0u8; 4];
    r.read_exact(&mut nr)?;
    let n_rg = u32::from_le_bytes(nr) as usize;
    let mut filters = HashMap::with_capacity(n_entries);
    for _ in 0..n_entries {
        let mut vl = [0u8; 4];
        r.read_exact(&mut vl)?;
        let vlen = u32::from_le_bytes(vl) as usize;
        let mut vbuf = vec![0u8; vlen];
        r.read_exact(&mut vbuf)?;
        let val = String::from_utf8(vbuf).map_err(|e| EngineError::Query(e.to_string()))?;
        let mut wl = [0u8; 4];
        r.read_exact(&mut wl)?;
        let wlen = u32::from_le_bytes(wl) as usize;
        let mut bitmask = vec![0u64; wlen];
        for w in &mut bitmask {
            let mut wb = [0u8; 8];
            r.read_exact(&mut wb)?;
            *w = u64::from_le_bytes(wb);
        }
        filters.insert(val, bitmask);
    }
    Ok(RoaringIndex {
        filters,
        n_row_groups: n_rg,
    })
}

pub fn applicable_predicate(pred: &Predicate) -> Option<(&str, Vec<&str>)> {
    match pred {
        Predicate::Eq {
            column,
            value: Literal::Text(s),
        } => Some((column.as_str(), vec![s.as_str()])),
        Predicate::In { column, values } => {
            let texts: Vec<&str> = values
                .iter()
                .filter_map(|v| {
                    if let Literal::Text(s) = v {
                        Some(s.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some((column.as_str(), texts))
            }
        }
        _ => None,
    }
}
