use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::arrow::ProjectionMask;

use crate::error::EngineError;

const MAGIC: u32 = 0x54565353;
const VERSION: u16 = 1;

pub struct SparseSortEntry {
    pub rg_idx: u32,
    pub row_start: u64,
    pub row_end: u64,
}

pub struct SparseSortIndex {
    pub entries: Vec<SparseSortEntry>,
    pub total_rows: u64,
    pub spill_path: PathBuf,
}

impl SparseSortIndex {
    pub fn byte_size(&self) -> u64 {
        (self.entries.capacity() * std::mem::size_of::<SparseSortEntry>()) as u64
    }
}

pub fn build_sparse(spill_path: &Path, index_path: &Path) -> Result<SparseSortIndex, EngineError> {
    let file = File::open(spill_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let metadata = builder.metadata();
    let n_rgs = metadata.num_row_groups();
    let total_rows = metadata.file_metadata().num_rows() as u64;

    let mut entries: Vec<SparseSortEntry> = Vec::with_capacity(n_rgs);
    let mut row_cursor = 0u64;
    for rg_idx in 0..n_rgs {
        let rg = metadata.row_group(rg_idx);
        let rg_rows = rg.num_rows() as u64;
        entries.push(SparseSortEntry {
            rg_idx: rg_idx as u32,
            row_start: row_cursor,
            row_end: row_cursor + rg_rows,
        });
        row_cursor += rg_rows;
    }

    let out = File::create(index_path)?;
    let mut w = BufWriter::new(out);
    w.write_all(&MAGIC.to_le_bytes())?;
    w.write_all(&VERSION.to_le_bytes())?;
    w.write_all(&total_rows.to_le_bytes())?;
    let n = entries.len() as u32;
    w.write_all(&n.to_le_bytes())?;
    for e in &entries {
        w.write_all(&e.rg_idx.to_le_bytes())?;
        w.write_all(&e.row_start.to_le_bytes())?;
        w.write_all(&e.row_end.to_le_bytes())?;
    }
    w.flush()?;

    Ok(SparseSortIndex {
        entries,
        total_rows,
        spill_path: spill_path.to_path_buf(),
    })
}

pub fn load_sparse(index_path: &Path, spill_path: PathBuf) -> Result<SparseSortIndex, EngineError> {
    let file = File::open(index_path)?;
    let mut r = BufReader::new(file);

    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if u32::from_le_bytes(magic) != MAGIC {
        return Err(EngineError::Query("invalid sparse sort index magic".into()));
    }
    let mut ver = [0u8; 2];
    r.read_exact(&mut ver)?;

    let mut total_rows_buf = [0u8; 8];
    r.read_exact(&mut total_rows_buf)?;
    let total_rows = u64::from_le_bytes(total_rows_buf);

    let mut n_buf = [0u8; 4];
    r.read_exact(&mut n_buf)?;
    let n = u32::from_le_bytes(n_buf) as usize;

    let mut entries = Vec::with_capacity(n);
    for _ in 0..n {
        let mut rg_buf = [0u8; 4];
        r.read_exact(&mut rg_buf)?;
        let mut rs_buf = [0u8; 8];
        r.read_exact(&mut rs_buf)?;
        let mut re_buf = [0u8; 8];
        r.read_exact(&mut re_buf)?;
        entries.push(SparseSortEntry {
            rg_idx: u32::from_le_bytes(rg_buf),
            row_start: u64::from_le_bytes(rs_buf),
            row_end: u64::from_le_bytes(re_buf),
        });
    }

    Ok(SparseSortIndex {
        entries,
        total_rows,
        spill_path,
    })
}

pub fn sparse_tile_lookup(
    index: &SparseSortIndex,
    row_offset: usize,
    limit: usize,
) -> Vec<(usize, usize, usize)> {
    let row_offset = row_offset as u64;
    let row_end = row_offset + limit as u64;
    let mut result = Vec::new();

    let start = index.entries.partition_point(|e| e.row_end <= row_offset);

    for e in &index.entries[start..] {
        if e.row_start >= row_end {
            break;
        }
        let within_rg_offset = if e.row_start < row_offset {
            (row_offset - e.row_start) as usize
        } else {
            0
        };
        let available = (e.row_end.min(row_end) - e.row_start.max(row_offset)) as usize;
        result.push((e.rg_idx as usize, within_rg_offset, available));
    }
    result
}

pub fn read_sparse_tile(
    spill_path: &Path,
    lookups: &[(usize, usize, usize)],
    col_offset: usize,
    cols: usize,
    schema: &SchemaRef,
) -> Result<Vec<RecordBatch>, EngineError> {
    if lookups.is_empty() {
        return Ok(vec![]);
    }
    let n_cols = schema.fields().len();
    let col_end = (col_offset + cols).min(n_cols);
    let col_indices: Vec<usize> = (col_offset..col_end).collect();

    let pq_meta = {
        let file = File::open(spill_path)?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
        builder.metadata().clone()
    };
    let arm_base = Arc::new(pq_meta);

    let mut result = Vec::new();
    for &(rg_idx, within_offset, take) in lookups {
        let file = File::open(spill_path)?;
        let arm =
            ArrowReaderMetadata::try_new(Arc::clone(&arm_base), ArrowReaderOptions::default())?;
        let builder = ParquetRecordBatchReaderBuilder::new_with_metadata(file, arm);
        let mask = ProjectionMask::roots(builder.parquet_schema(), col_indices.iter().copied());
        let reader = builder
            .with_projection(mask)
            .with_row_groups(vec![rg_idx])
            .with_batch_size(4096)
            .build()
            .map_err(EngineError::Parquet)?;

        let mut skipped = 0usize;
        let mut collected = 0usize;
        for batch_result in reader {
            if collected >= take {
                break;
            }
            let batch = batch_result.map_err(EngineError::Arrow)?;
            let n = batch.num_rows();
            if skipped + n <= within_offset {
                skipped += n;
                continue;
            }
            let start = within_offset.saturating_sub(skipped);
            let available = n - start;
            let to_take = available.min(take - collected);
            result.push(batch.slice(start, to_take));
            skipped += n;
            collected += to_take;
        }
    }
    Ok(result)
}
