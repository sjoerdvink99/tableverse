use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::statistics::Statistics;

use crate::error::EngineError;

const MAGIC: [u8; 4] = *b"TVMK";
const VERSION: u32 = 1;

#[derive(Clone, Debug)]
pub struct MarkEntry {
    pub min_key: f64,
    pub max_key: f64,
    pub row_group_idx: u32,
    pub row_start: u64,
    pub row_count: u64,
}

pub struct MarkIndex {
    entries: Vec<MarkEntry>,
}

impl MarkIndex {
    pub fn new(entries: Vec<MarkEntry>) -> Self {
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn lookup_eq(&self, val: f64) -> Vec<usize> {
        self.entries
            .iter()
            .filter(|e| e.min_key <= val && val <= e.max_key)
            .map(|e| e.row_group_idx as usize)
            .collect()
    }

    pub fn lookup_gt(&self, val: f64) -> Vec<usize> {
        self.entries
            .iter()
            .filter(|e| e.max_key > val)
            .map(|e| e.row_group_idx as usize)
            .collect()
    }

    pub fn lookup_gte(&self, val: f64) -> Vec<usize> {
        self.entries
            .iter()
            .filter(|e| e.max_key >= val)
            .map(|e| e.row_group_idx as usize)
            .collect()
    }

    pub fn lookup_lt(&self, val: f64) -> Vec<usize> {
        self.entries
            .iter()
            .filter(|e| e.min_key < val)
            .map(|e| e.row_group_idx as usize)
            .collect()
    }

    pub fn lookup_lte(&self, val: f64) -> Vec<usize> {
        self.entries
            .iter()
            .filter(|e| e.min_key <= val)
            .map(|e| e.row_group_idx as usize)
            .collect()
    }

    pub fn lookup_between(&self, lo: f64, hi: f64) -> Vec<usize> {
        self.entries
            .iter()
            .filter(|e| e.max_key >= lo && e.min_key <= hi)
            .map(|e| e.row_group_idx as usize)
            .collect()
    }

    pub fn save(&self, path: &str) -> Result<(), EngineError> {
        let f = File::create(path)?;
        let mut w = BufWriter::new(f);
        w.write_all(&MAGIC)?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&(self.entries.len() as u64).to_le_bytes())?;
        for e in &self.entries {
            w.write_all(&e.min_key.to_le_bytes())?;
            w.write_all(&e.max_key.to_le_bytes())?;
            w.write_all(&e.row_group_idx.to_le_bytes())?;
            w.write_all(&e.row_start.to_le_bytes())?;
            w.write_all(&e.row_count.to_le_bytes())?;
        }
        w.flush()?;
        Ok(())
    }

    pub fn load(path: &str) -> Result<Self, EngineError> {
        let f = File::open(path)?;
        let mut r = BufReader::new(f);
        let mut magic = [0u8; 4];
        r.read_exact(&mut magic)?;
        if magic != MAGIC {
            return Err(EngineError::Query("invalid mark index magic".into()));
        }
        let mut ver_buf = [0u8; 4];
        r.read_exact(&mut ver_buf)?;
        let version = u32::from_le_bytes(ver_buf);
        if version != VERSION {
            return Err(EngineError::Query(format!(
                "unsupported mark index version {version}"
            )));
        }
        let mut n_buf = [0u8; 8];
        r.read_exact(&mut n_buf)?;
        let n = u64::from_le_bytes(n_buf) as usize;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let mut buf = [0u8; 8];
            r.read_exact(&mut buf)?;
            let min_key = f64::from_le_bytes(buf);
            r.read_exact(&mut buf)?;
            let max_key = f64::from_le_bytes(buf);
            let mut u32_buf = [0u8; 4];
            r.read_exact(&mut u32_buf)?;
            let row_group_idx = u32::from_le_bytes(u32_buf);
            r.read_exact(&mut buf)?;
            let row_start = u64::from_le_bytes(buf);
            r.read_exact(&mut buf)?;
            let row_count = u64::from_le_bytes(buf);
            entries.push(MarkEntry {
                min_key,
                max_key,
                row_group_idx,
                row_start,
                row_count,
            });
        }
        Ok(Self { entries })
    }
}

fn stats_to_f64(stats: &Statistics) -> (Option<f64>, Option<f64>) {
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

pub fn mark_index_path(source_path: &str, col_idx: usize) -> PathBuf {
    PathBuf::from(format!("{source_path}.col_{col_idx}.tvk"))
}

pub fn build_mark_index(source_path: &str, col_idx: usize) -> Result<MarkIndex, EngineError> {
    let file = File::open(source_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let metadata = builder.metadata();
    let n_rgs = metadata.num_row_groups();

    let mut entries = Vec::with_capacity(n_rgs);
    let mut row_start = 0u64;

    for rg_idx in 0..n_rgs {
        let rg = metadata.row_group(rg_idx);
        let row_count = rg.num_rows() as u64;
        let (min_key, max_key) = if col_idx < rg.num_columns() {
            rg.column(col_idx)
                .statistics()
                .map(|s| {
                    let (mn, mx) = stats_to_f64(s);
                    (mn.unwrap_or(f64::NEG_INFINITY), mx.unwrap_or(f64::INFINITY))
                })
                .unwrap_or((f64::NEG_INFINITY, f64::INFINITY))
        } else {
            (f64::NEG_INFINITY, f64::INFINITY)
        };
        entries.push(MarkEntry {
            min_key,
            max_key,
            row_group_idx: rg_idx as u32,
            row_start,
            row_count,
        });
        row_start += row_count;
    }

    Ok(MarkIndex::new(entries))
}

pub type MarkCache = Arc<RwLock<std::collections::HashMap<(String, usize), Arc<MarkIndex>>>>;

pub fn new_mark_cache() -> MarkCache {
    Arc::new(RwLock::new(std::collections::HashMap::new()))
}
