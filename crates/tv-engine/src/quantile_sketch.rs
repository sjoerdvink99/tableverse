use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use arrow::array::Array;
use arrow::compute::cast;
use arrow::datatypes::{DataType as ArrowDataType, SchemaRef};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ProjectionMask;

use crate::error::EngineError;

const MAGIC: u32 = 0x54565100;
const VERSION: u16 = 1;

#[derive(Clone)]
pub struct Centroid {
    pub mean: f64,
    pub weight: f64,
}

#[derive(Clone)]
pub struct TDigest {
    pub centroids: Vec<Centroid>,
    pub n: u64,
    pub compression: f64,
}

impl TDigest {
    pub fn new(compression: f64) -> Self {
        Self {
            centroids: Vec::new(),
            n: 0,
            compression,
        }
    }

    pub fn add(&mut self, value: f64) {
        self.n += 1;
        let mut min_dist = f64::MAX;
        let mut closest = self.centroids.len();
        for (i, c) in self.centroids.iter().enumerate() {
            let d = (c.mean - value).abs();
            if d < min_dist {
                min_dist = d;
                closest = i;
            }
        }
        let limit = 4.0 * self.compression * self.n as f64 / self.centroids.len().max(1) as f64;
        if closest < self.centroids.len() && self.centroids[closest].weight < limit {
            let c = &mut self.centroids[closest];
            let w = c.weight + 1.0;
            c.mean = (c.mean * c.weight + value) / w;
            c.weight = w;
        } else {
            self.centroids.push(Centroid {
                mean: value,
                weight: 1.0,
            });
            if self.centroids.len() > (self.compression as usize * 2).max(32) {
                self.compress();
            }
        }
    }

    fn compress(&mut self) {
        self.centroids.sort_by(|a, b| {
            a.mean
                .partial_cmp(&b.mean)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut merged: Vec<Centroid> = Vec::new();
        let total_w: f64 = self.centroids.iter().map(|c| c.weight).sum();
        let mut cum_w = 0.0f64;
        for c in &self.centroids {
            let q = (cum_w + c.weight / 2.0) / total_w;
            let limit = 4.0 * self.compression * q * (1.0 - q);
            if let Some(last) = merged.last_mut() {
                if last.weight + c.weight <= limit {
                    let w = last.weight + c.weight;
                    last.mean = (last.mean * last.weight + c.mean * c.weight) / w;
                    last.weight = w;
                    cum_w += c.weight;
                    continue;
                }
            }
            merged.push(c.clone());
            cum_w += c.weight;
        }
        self.centroids = merged;
    }

    pub fn merge(&mut self, other: &TDigest) {
        for c in &other.centroids {
            for _ in 0..(c.weight as u64).max(1) {
                self.add(c.mean);
            }
        }
        self.n += other.n;
    }

    pub fn quantile(&self, q: f64) -> f64 {
        if self.centroids.is_empty() {
            return f64::NAN;
        }
        let total_w: f64 = self.centroids.iter().map(|c| c.weight).sum();
        let target = q * total_w;
        let mut cum = 0.0f64;
        let mut sorted = self.centroids.clone();
        sorted.sort_by(|a, b| {
            a.mean
                .partial_cmp(&b.mean)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for c in &sorted {
            cum += c.weight;
            if cum >= target {
                return c.mean;
            }
        }
        sorted.last().map(|c| c.mean).unwrap_or(f64::NAN)
    }

    pub fn cdf(&self, x: f64) -> f64 {
        if self.centroids.is_empty() {
            return 0.5;
        }
        let mut sorted = self.centroids.clone();
        sorted.sort_by(|a, b| {
            a.mean
                .partial_cmp(&b.mean)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let total_w: f64 = sorted.iter().map(|c| c.weight).sum();
        let mut cum = 0.0f64;
        for c in &sorted {
            if c.mean > x {
                break;
            }
            cum += c.weight;
        }
        (cum / total_w).clamp(0.0, 1.0)
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.n.to_le_bytes());
        buf.extend_from_slice(&self.compression.to_le_bytes());
        let n = self.centroids.len() as u32;
        buf.extend_from_slice(&n.to_le_bytes());
        for c in &self.centroids {
            buf.extend_from_slice(&c.mean.to_le_bytes());
            buf.extend_from_slice(&c.weight.to_le_bytes());
        }
        buf
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, EngineError> {
        if bytes.len() < 20 {
            return Err(EngineError::Query("t-digest too short".into()));
        }
        let n = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let compression = f64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let nc = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        let mut centroids = Vec::with_capacity(nc);
        let mut pos = 20;
        for _ in 0..nc {
            if pos + 16 > bytes.len() {
                return Err(EngineError::Query("t-digest data truncated".into()));
            }
            let mean = f64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap());
            let weight = f64::from_le_bytes(bytes[pos + 8..pos + 16].try_into().unwrap());
            centroids.push(Centroid { mean, weight });
            pos += 16;
        }
        Ok(Self {
            centroids,
            n,
            compression,
        })
    }
}

pub struct QuantileIndex {
    pub sketches: HashMap<(usize, usize), TDigest>,
    pub global: HashMap<usize, TDigest>,
}

pub fn build_quantile_index(
    source_path: &str,
    col_indices: &[usize],
    index_path: &Path,
) -> Result<QuantileIndex, EngineError> {
    let file = File::open(source_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let schema: SchemaRef = builder.schema().clone();
    let n_rgs = builder.metadata().num_row_groups();
    drop(builder);

    let mut sketches: HashMap<(usize, usize), TDigest> = HashMap::new();
    let mut global: HashMap<usize, TDigest> = HashMap::new();

    let numeric_cols: Vec<usize> = col_indices
        .iter()
        .copied()
        .filter(|&i| {
            if i >= schema.fields().len() {
                return false;
            }
            matches!(
                schema.field(i).data_type(),
                ArrowDataType::Float32
                    | ArrowDataType::Float64
                    | ArrowDataType::Int8
                    | ArrowDataType::Int16
                    | ArrowDataType::Int32
                    | ArrowDataType::Int64
                    | ArrowDataType::UInt8
                    | ArrowDataType::UInt16
                    | ArrowDataType::UInt32
                    | ArrowDataType::UInt64
            )
        })
        .collect();

    for rg_idx in 0..n_rgs {
        for &col_idx in &numeric_cols {
            let file = File::open(source_path)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
            let mask = ProjectionMask::roots(builder.parquet_schema(), [col_idx]);
            let reader = builder
                .with_projection(mask)
                .with_row_groups(vec![rg_idx])
                .build()
                .map_err(EngineError::Parquet)?;

            let mut sketch = TDigest::new(100.0);
            for batch_result in reader {
                let batch = batch_result.map_err(EngineError::Arrow)?;
                let col = batch.column(0);
                if let Ok(fc) = cast(col.as_ref(), &ArrowDataType::Float64) {
                    if let Some(arr) = fc.as_any().downcast_ref::<arrow::array::Float64Array>() {
                        for i in 0..arr.len() {
                            if !arr.is_null(i) {
                                let v = arr.value(i);
                                if v.is_finite() {
                                    sketch.add(v);
                                }
                            }
                        }
                    }
                }
            }

            let g = global.entry(col_idx).or_insert_with(|| TDigest::new(100.0));
            g.merge(&sketch);
            sketches.insert((rg_idx, col_idx), sketch);
        }
    }

    let out = File::create(index_path)?;
    let mut w = BufWriter::new(out);
    w.write_all(&MAGIC.to_le_bytes())?;
    w.write_all(&VERSION.to_le_bytes())?;
    let n_entries = sketches.len() as u32;
    w.write_all(&n_entries.to_le_bytes())?;
    for (&(rg, col), sketch) in &sketches {
        w.write_all(&(rg as u32).to_le_bytes())?;
        w.write_all(&(col as u32).to_le_bytes())?;
        let bytes = sketch.serialize();
        w.write_all(&(bytes.len() as u32).to_le_bytes())?;
        w.write_all(&bytes)?;
    }
    let n_global = global.len() as u32;
    w.write_all(&n_global.to_le_bytes())?;
    for (&col, sketch) in &global {
        w.write_all(&(col as u32).to_le_bytes())?;
        let bytes = sketch.serialize();
        w.write_all(&(bytes.len() as u32).to_le_bytes())?;
        w.write_all(&bytes)?;
    }
    w.flush()?;

    Ok(QuantileIndex { sketches, global })
}

pub fn tdigest_to_global_sketches(
    index: &QuantileIndex,
    schema: &SchemaRef,
) -> HashMap<String, tv_core::types::QuantileSketch> {
    let mut result = HashMap::new();
    for (&col_idx, td) in &index.global {
        if let Some(field) = schema.fields().get(col_idx) {
            let values: Vec<f64> = (0..=200)
                .map(|i| td.quantile(i as f64 / 200.0))
                .filter(|v| v.is_finite())
                .collect();
            result.insert(
                field.name().clone(),
                tv_core::types::QuantileSketch::new(values),
            );
        }
    }
    result
}

pub fn load_quantile_index(index_path: &Path) -> Result<QuantileIndex, EngineError> {
    let file = File::open(index_path)?;
    let mut r = BufReader::new(file);

    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if u32::from_le_bytes(magic) != MAGIC {
        return Err(EngineError::Query("invalid quantile index magic".into()));
    }
    let mut ver = [0u8; 2];
    r.read_exact(&mut ver)?;

    let mut n_buf = [0u8; 4];
    r.read_exact(&mut n_buf)?;
    let n_entries = u32::from_le_bytes(n_buf) as usize;

    let mut sketches = HashMap::new();
    for _ in 0..n_entries {
        let mut rg_buf = [0u8; 4];
        r.read_exact(&mut rg_buf)?;
        let mut col_buf = [0u8; 4];
        r.read_exact(&mut col_buf)?;
        let mut len_buf = [0u8; 4];
        r.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut bytes = vec![0u8; len];
        r.read_exact(&mut bytes)?;
        let rg = u32::from_le_bytes(rg_buf) as usize;
        let col = u32::from_le_bytes(col_buf) as usize;
        sketches.insert((rg, col), TDigest::deserialize(&bytes)?);
    }

    let mut ng_buf = [0u8; 4];
    r.read_exact(&mut ng_buf)?;
    let n_global = u32::from_le_bytes(ng_buf) as usize;
    let mut global = HashMap::new();
    for _ in 0..n_global {
        let mut col_buf = [0u8; 4];
        r.read_exact(&mut col_buf)?;
        let mut len_buf = [0u8; 4];
        r.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut bytes = vec![0u8; len];
        r.read_exact(&mut bytes)?;
        let col = u32::from_le_bytes(col_buf) as usize;
        global.insert(col, TDigest::deserialize(&bytes)?);
    }

    Ok(QuantileIndex { sketches, global })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::write_test_parquet;
    use tempfile::TempDir;

    fn make_dir() -> TempDir {
        tempfile::TempDir::new().unwrap()
    }

    fn uniform_digest(n: usize) -> TDigest {
        let mut td = TDigest::new(100.0);
        for i in 0..n {
            td.add(i as f64);
        }
        td
    }

    #[test]
    fn tdigest_quantile_median_uniform() {
        let td = uniform_digest(100);
        let median = td.quantile(0.5);
        assert!(
            (median - 50.0).abs() < 5.0,
            "median {median} should be near 50"
        );
    }

    #[test]
    fn tdigest_quantile_extreme_percentiles() {
        let td = uniform_digest(100);
        let p25 = td.quantile(0.25);
        let p75 = td.quantile(0.75);
        let p99 = td.quantile(0.99);
        assert!(p25 <= p75, "p25 {p25} should be <= p75 {p75}");
        assert!(p75 <= p99, "p75 {p75} should be <= p99 {p99}");
        assert!(
            p25 >= 0.0 && p99 <= 99.0,
            "quantiles should be within data range"
        );
    }

    #[test]
    fn tdigest_cdf_monotonic() {
        let td = uniform_digest(200);
        let xs: Vec<f64> = (0..10).map(|i| i as f64 * 20.0).collect();
        for w in xs.windows(2) {
            let a = w[0];
            let b = w[1];
            assert!(
                td.cdf(a) <= td.cdf(b),
                "cdf({a}) > cdf({b}) — not monotonic"
            );
        }
    }

    #[test]
    fn tdigest_serialize_deserialize_roundtrip() {
        let td = uniform_digest(100);
        let q50_before = td.quantile(0.5);
        let q90_before = td.quantile(0.9);

        let bytes = td.serialize();
        let td2 = TDigest::deserialize(&bytes).unwrap();

        let q50_after = td2.quantile(0.5);
        let q90_after = td2.quantile(0.9);

        assert!(
            (q50_before - q50_after).abs() < 1e-9,
            "q50 mismatch after roundtrip"
        );
        assert!(
            (q90_before - q90_after).abs() < 1e-9,
            "q90 mismatch after roundtrip"
        );
    }

    #[test]
    fn build_quantile_index_roundtrip() {
        let dir = make_dir();
        let batches = crate::test_helpers::numbers_batches();
        let parquet_path = write_test_parquet(&dir, "nums.parquet", &batches);
        let index_path = dir.path().join("nums.tvq");

        let index =
            build_quantile_index(parquet_path.to_str().unwrap(), &[0, 1], &index_path).unwrap();

        assert!(!index.global.is_empty());

        let loaded = load_quantile_index(&index_path).unwrap();
        assert_eq!(loaded.global.len(), index.global.len());

        for (&col_idx, orig_td) in &index.global {
            let loaded_td = loaded.global.get(&col_idx).unwrap();
            let q50_orig = orig_td.quantile(0.5);
            let q50_load = loaded_td.quantile(0.5);
            assert!(
                (q50_orig - q50_load).abs() < 1e-9,
                "q50 mismatch for col {col_idx} after load"
            );
        }
    }
}
