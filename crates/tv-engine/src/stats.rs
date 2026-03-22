use std::collections::HashMap;
use std::fs::File;

use arrow::array::{Array, ArrayRef, Float64Array, StringArray};
use arrow::compute::cast;
use arrow::datatypes::DataType as ArrowDataType;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::arrow::ProjectionMask;
use tv_core::{
    CardinalityCategory, ColumnInfo, ColumnStats, CorrelationMatrix, HistogramBucket, Quantiles,
    QuickColumnStats, SourceMeta, TopValue,
};

use crate::error::EngineError;
use crate::reader;

pub(crate) const DEFAULT_HISTOGRAM_BINS: usize = 50;

type StreamingStatsResult = Result<
    (
        u64,
        Option<f64>,
        Option<Vec<HistogramBucket>>,
        Option<Quantiles>,
        Option<Vec<TopValue>>,
    ),
    EngineError,
>;

struct HyperLogLog {
    registers: [u8; 16384],
}

impl HyperLogLog {
    fn new() -> Self {
        Self {
            registers: [0u8; 16384],
        }
    }

    fn add_hash(&mut self, hash: u64) {
        let idx = (hash >> 50) as usize;
        let rest = hash << 14;
        let leading = rest.leading_zeros() as u8 + 1;
        if leading > self.registers[idx] {
            self.registers[idx] = leading;
        }
    }

    fn add_str(&mut self, s: &str) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        s.hash(&mut h);
        self.add_hash(h.finish());
    }

    fn add_f64(&mut self, val: f64) {
        self.add_hash(mix64(val.to_bits()));
    }

    fn add_null(&mut self) {
        self.add_hash(u64::MAX);
    }

    fn merge(&mut self, other: &HyperLogLog) {
        for (r, o) in self.registers.iter_mut().zip(other.registers.iter()) {
            if *o > *r {
                *r = *o;
            }
        }
    }

    fn estimate(&self) -> u64 {
        const M: f64 = 16384.0;
        const ALPHA_M_SQ: f64 = 0.7213 / (1.0 + 1.079 / M) * M * M;

        let z: f64 = self
            .registers
            .iter()
            .map(|&r| 2.0f64.powi(-(r as i32)))
            .sum();
        let e = ALPHA_M_SQ / z;

        if e <= 2.5 * M {
            let v = self.registers.iter().filter(|&&r| r == 0).count() as f64;
            if v > 0.0 {
                (M * (M / v).ln()) as u64
            } else {
                e as u64
            }
        } else if e <= 143_165_576.0 {
            e as u64
        } else {
            (-(4_294_967_296.0_f64) * (1.0 - e / 4_294_967_296.0).ln()) as u64
        }
    }
}

struct WelfordPair {
    n: u64,
    mean_x: f64,
    mean_y: f64,
    m2_x: f64,
    m2_y: f64,
    co_moment: f64,
}

impl WelfordPair {
    fn new() -> Self {
        Self {
            n: 0,
            mean_x: 0.0,
            mean_y: 0.0,
            m2_x: 0.0,
            m2_y: 0.0,
            co_moment: 0.0,
        }
    }

    fn update(&mut self, x: f64, y: f64) {
        self.n += 1;
        let n = self.n as f64;
        let dx = x - self.mean_x;
        let dy = y - self.mean_y;
        self.mean_x += dx / n;
        self.mean_y += dy / n;
        self.m2_x += dx * (x - self.mean_x);
        self.m2_y += dy * (y - self.mean_y);
        self.co_moment += dx * (y - self.mean_y);
    }

    fn correlation(&self) -> Option<f64> {
        if self.n < 2 || self.m2_x == 0.0 || self.m2_y == 0.0 {
            return None;
        }
        let r = self.co_moment / (self.m2_x.sqrt() * self.m2_y.sqrt());
        Some(r.clamp(-1.0, 1.0))
    }
}

const RESERVOIR_CAPACITY: usize = 8192;
const TOP_VALUES_MAX_DISTINCT: usize = 200;
const TOP_VALUES_RETURN_K: usize = 10;
const N_SAMPLE_RGS: usize = 32;

struct ReservoirSampler {
    samples: Vec<f64>,
    count: u64,
}

impl ReservoirSampler {
    fn new() -> Self {
        Self {
            samples: Vec::with_capacity(RESERVOIR_CAPACITY),
            count: 0,
        }
    }

    fn add(&mut self, v: f64) {
        self.count += 1;
        if self.samples.len() < RESERVOIR_CAPACITY {
            self.samples.push(v);
        } else {
            let j = mix64(self.count) % self.count;
            if j < RESERVOIR_CAPACITY as u64 {
                self.samples[j as usize] = v;
            }
        }
    }

    fn merge(&mut self, other: ReservoirSampler) {
        self.count += other.count;
        self.samples.extend(other.samples);
        if self.samples.len() > RESERVOIR_CAPACITY {
            let total = self.samples.len();
            let mut rng = mix64(self.count);
            for i in 0..RESERVOIR_CAPACITY {
                rng = mix64(rng);
                let j = i + (rng as usize) % (total - i);
                self.samples.swap(i, j);
            }
            self.samples.truncate(RESERVOIR_CAPACITY);
        }
    }

    fn quantiles(&mut self) -> Option<Quantiles> {
        if self.samples.len() < 2 {
            return None;
        }
        self.samples
            .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = self.samples.len();
        let q = |p: f64| {
            let idx = (p * (n - 1) as f64).round() as usize;
            self.samples[idx.min(n - 1)]
        };
        Some(Quantiles {
            p1: q(0.01),
            p5: q(0.05),
            p25: q(0.25),
            p50: q(0.50),
            p75: q(0.75),
            p95: q(0.95),
            p99: q(0.99),
        })
    }
}

fn mix64(x: u64) -> u64 {
    let x = x.wrapping_mul(0x9e3779b97f4a7c15);
    let x = x ^ (x >> 30);
    let x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    let x = x ^ (x >> 27);
    let x = x.wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

struct TopValuesTracker {
    counts: HashMap<String, u64>,
    overflowed: bool,
}

impl TopValuesTracker {
    fn new() -> Self {
        Self {
            counts: HashMap::new(),
            overflowed: false,
        }
    }

    fn add(&mut self, v: &str) {
        if self.overflowed {
            return;
        }
        if !self.counts.contains_key(v) && self.counts.len() >= TOP_VALUES_MAX_DISTINCT {
            self.overflowed = true;
            return;
        }
        *self.counts.entry(v.to_string()).or_insert(0) += 1;
    }

    fn merge(&mut self, other: TopValuesTracker) {
        if self.overflowed {
            return;
        }
        if other.overflowed {
            self.overflowed = true;
            return;
        }
        for (k, v) in other.counts {
            if !self.counts.contains_key(&k) && self.counts.len() >= TOP_VALUES_MAX_DISTINCT {
                self.overflowed = true;
                return;
            }
            *self.counts.entry(k).or_insert(0) += v;
        }
    }

    fn top_k(self, total: u64) -> Option<Vec<TopValue>> {
        if self.overflowed {
            return None;
        }
        let mut pairs: Vec<(String, u64)> = self.counts.into_iter().collect();
        pairs.sort_by_key(|b| std::cmp::Reverse(b.1));
        pairs.truncate(TOP_VALUES_RETURN_K);
        Some(
            pairs
                .into_iter()
                .map(|(v, count)| TopValue {
                    value: serde_json::Value::String(v),
                    count,
                    rate: if total > 0 {
                        count as f64 / total as f64
                    } else {
                        0.0
                    },
                })
                .collect(),
        )
    }
}

struct BatchAcc {
    hll: HyperLogLog,
    sum: f64,
    count: u64,
    histogram_counts: Vec<u64>,
    reservoir: ReservoirSampler,
    top_tracker: TopValuesTracker,
}

impl BatchAcc {
    fn new(n_bins: usize) -> Self {
        Self {
            hll: HyperLogLog::new(),
            sum: 0.0,
            count: 0,
            histogram_counts: vec![0u64; n_bins],
            reservoir: ReservoirSampler::new(),
            top_tracker: TopValuesTracker::new(),
        }
    }

    fn merge(&mut self, other: BatchAcc) {
        self.hll.merge(&other.hll);
        self.sum += other.sum;
        self.count += other.count;
        for (a, b) in self
            .histogram_counts
            .iter_mut()
            .zip(other.histogram_counts.iter())
        {
            *a += b;
        }
        self.reservoir.merge(other.reservoir);
        self.top_tracker.merge(other.top_tracker);
    }
}

fn process_batch_stats(
    batch: &arrow::record_batch::RecordBatch,
    is_numeric: bool,
    histo_lo: f64,
    has_range: bool,
    bin_width: f64,
    n_bins: usize,
) -> Result<BatchAcc, EngineError> {
    let mut acc = BatchAcc::new(n_bins);
    let arr = batch.column(0);

    if is_numeric {
        let f64_col = cast(arr.as_ref(), &ArrowDataType::Float64)?;
        let f64_arr = f64_col
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| EngineError::Query("float64 cast failed".into()))?;
        for i in 0..f64_arr.len() {
            if f64_arr.is_null(i) {
                acc.hll.add_null();
            } else {
                let v = f64_arr.value(i);
                acc.hll.add_f64(v);
                if v.is_finite() {
                    acc.sum += v;
                    acc.count += 1;
                    acc.reservoir.add(v);
                    if has_range {
                        let bin = ((v - histo_lo) / bin_width) as usize;
                        acc.histogram_counts[bin.min(n_bins - 1)] += 1;
                    }
                }
            }
        }
    } else {
        let str_col = match cast(arr.as_ref(), &ArrowDataType::Utf8) {
            Ok(c) => c,
            Err(_) => {
                for i in 0..arr.len() {
                    if arr.is_null(i) {
                        acc.hll.add_null();
                    }
                }
                return Ok(acc);
            }
        };
        if let Some(s) = str_col.as_any().downcast_ref::<StringArray>() {
            for i in 0..s.len() {
                if s.is_null(i) {
                    acc.hll.add_null();
                } else {
                    let v = s.value(i);
                    acc.hll.add_str(v);
                    acc.top_tracker.add(v);
                }
            }
        }
    }
    Ok(acc)
}

fn classify_cardinality(distinct: u64, total: u64) -> CardinalityCategory {
    if total == 0 {
        return CardinalityCategory::Unknown;
    }
    if distinct == 0 {
        return CardinalityCategory::Unknown;
    }
    if distinct == 1 {
        return CardinalityCategory::Constant;
    }
    if distinct == 2 {
        return CardinalityCategory::Binary;
    }
    if distinct <= 20 {
        return CardinalityCategory::LowCardinality;
    }
    let sqrt_total = (total as f64).sqrt() as u64;
    if distinct <= sqrt_total {
        return CardinalityCategory::Categorical;
    }
    if distinct as f64 >= total as f64 * 0.95 {
        return CardinalityCategory::Unique;
    }
    CardinalityCategory::HighCardinality
}

pub fn compute_column_stats(
    meta: &SourceMeta,
    col_idx: usize,
    n_bins: usize,
) -> Result<ColumnStats, EngineError> {
    let col = meta
        .columns
        .get(col_idx)
        .ok_or_else(|| EngineError::SourceNotFound(format!("column index {col_idx}")))?;

    if matches!(meta.format, tv_core::SourceFormat::Parquet)
        && !matches!(
            meta.kind,
            tv_core::SourceKind::S3
                | tv_core::SourceKind::Gcs
                | tv_core::SourceKind::AzureBlob
                | tv_core::SourceKind::Http
        )
    {
        let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
        let quick = meta.quick_stats.get(col_idx);
        return compute_column_stats_parquet_streaming(
            col,
            col_idx,
            path,
            meta.n_rows,
            quick,
            n_bins,
        );
    }

    let batches = reader::read_parquet_full(&meta.uri, Some(&[col_idx]))?;
    let arrays: Vec<ArrayRef> = batches.iter().map(|b| b.column(0).clone()).collect();
    let total_count: u64 = arrays.iter().map(|a| a.len() as u64).sum();
    let null_count: u64 = arrays.iter().map(|a| a.null_count() as u64).sum();
    let null_rate = if total_count > 0 {
        null_count as f64 / total_count as f64
    } else {
        0.0
    };
    let distinct_count = compute_distinct_count_arrays(&arrays);
    let (min, max) = compute_min_max_arrays(&arrays, is_numeric_type(&col.data_type));
    let mean = if is_numeric_type(&col.data_type) {
        compute_mean_arrays(&arrays).ok()
    } else {
        None
    };
    let histogram = if is_numeric_type(&col.data_type) {
        compute_histogram_arrays(&arrays, n_bins).ok()
    } else {
        None
    };
    let (quantiles, top_values) =
        compute_quantiles_and_top_arrays(&arrays, is_numeric_type(&col.data_type), total_count);
    let cardinality_category = classify_cardinality(distinct_count, total_count);

    Ok(ColumnStats {
        column: col.name.clone(),
        index: col_idx,
        data_type: col.data_type.clone(),
        count: total_count,
        null_count,
        null_rate,
        distinct_count: Some(distinct_count),
        min,
        max,
        mean,
        quantiles,
        histogram,
        top_values,
        cardinality_category,
    })
}

pub fn compute_column_stats_from_batches(
    col: &ColumnInfo,
    col_idx: usize,
    batches: &[arrow::record_batch::RecordBatch],
    n_bins: usize,
) -> Result<ColumnStats, EngineError> {
    let arrays: Vec<ArrayRef> = batches.iter().map(|b| b.column(0).clone()).collect();
    let total_count: u64 = arrays.iter().map(|a| a.len() as u64).sum();
    let null_count: u64 = arrays.iter().map(|a| a.null_count() as u64).sum();
    let null_rate = if total_count > 0 {
        null_count as f64 / total_count as f64
    } else {
        0.0
    };
    let distinct_count = compute_distinct_count_arrays(&arrays);
    let is_numeric = is_numeric_type(&col.data_type);
    let (min, max) = compute_min_max_arrays(&arrays, is_numeric);
    let mean = if is_numeric {
        compute_mean_arrays(&arrays).ok()
    } else {
        None
    };
    let histogram = if is_numeric {
        compute_histogram_arrays(&arrays, n_bins).ok()
    } else {
        None
    };
    let (quantiles, top_values) =
        compute_quantiles_and_top_arrays(&arrays, is_numeric, total_count);
    let cardinality_category = classify_cardinality(distinct_count, total_count);

    Ok(ColumnStats {
        column: col.name.clone(),
        index: col_idx,
        data_type: col.data_type.clone(),
        count: total_count,
        null_count,
        null_rate,
        distinct_count: Some(distinct_count),
        min,
        max,
        mean,
        quantiles,
        histogram,
        top_values,
        cardinality_category,
    })
}

fn compute_column_stats_parquet_streaming(
    col: &ColumnInfo,
    col_idx: usize,
    path: &str,
    total_rows: u64,
    quick: Option<&QuickColumnStats>,
    n_bins: usize,
) -> Result<ColumnStats, EngineError> {
    let (meta_min, meta_max, null_count) = if let Some(q) = quick {
        (
            q.min.as_ref().and_then(|v| v.as_f64()),
            q.max.as_ref().and_then(|v| v.as_f64()),
            q.null_count,
        )
    } else {
        let (mn, mx, nc, _) = reader::parquet_column_stats_from_metadata(path, col_idx)?;
        (mn, mx, nc)
    };

    let null_rate = if total_rows > 0 {
        null_count as f64 / total_rows as f64
    } else {
        0.0
    };

    let is_numeric = is_numeric_type(&col.data_type);

    let (min, max) = if is_numeric {
        (
            meta_min.and_then(|f| serde_json::Number::from_f64(f).map(serde_json::Value::Number)),
            meta_max.and_then(|f| serde_json::Number::from_f64(f).map(serde_json::Value::Number)),
        )
    } else {
        (None, None)
    };

    let (distinct_count, mean, histogram, quantiles, top_values) = compute_streaming_stats(
        path, col_idx, is_numeric, meta_min, meta_max, total_rows, n_bins,
    )?;

    let cardinality_category = classify_cardinality(distinct_count, total_rows);

    Ok(ColumnStats {
        column: col.name.clone(),
        index: col_idx,
        data_type: col.data_type.clone(),
        count: total_rows,
        null_count,
        null_rate,
        distinct_count: Some(distinct_count),
        min,
        max,
        mean,
        quantiles,
        histogram,
        top_values,
        cardinality_category,
    })
}

fn sample_row_group_indices(n_total: usize, n_sample: usize) -> Vec<usize> {
    if n_sample >= n_total {
        return (0..n_total).collect();
    }
    (0..n_sample).map(|i| i * n_total / n_sample).collect()
}

fn compute_streaming_stats(
    path: &str,
    col_idx: usize,
    is_numeric: bool,
    meta_min: Option<f64>,
    meta_max: Option<f64>,
    total_rows: u64,
    n_bins: usize,
) -> StreamingStatsResult {
    let file = File::open(path)?;
    let metadata = ArrowReaderMetadata::load(&file, ArrowReaderOptions::new())
        .map_err(EngineError::Parquet)?;
    let n_rgs = metadata.metadata().num_row_groups();

    let rg_indices = sample_row_group_indices(n_rgs, N_SAMPLE_RGS.min(n_rgs));
    let n_sampled = rg_indices.len();

    let (histo_lo, histo_hi, has_range) = match (meta_min, meta_max) {
        (Some(lo), Some(hi)) if lo < hi => (lo, hi, true),
        _ => (0.0, 1.0, false),
    };
    let bin_width = if has_range {
        (histo_hi - histo_lo) / n_bins as f64
    } else {
        1.0
    };

    use rayon::prelude::*;
    let partials: Vec<BatchAcc> = rg_indices
        .par_iter()
        .map(|&rg_idx| {
            let batches = reader::read_parquet_row_group_column(path, col_idx, rg_idx, &metadata)?;
            let mut acc = BatchAcc::new(n_bins);
            for batch in &batches {
                let partial =
                    process_batch_stats(batch, is_numeric, histo_lo, has_range, bin_width, n_bins)?;
                acc.merge(partial);
            }
            Ok(acc)
        })
        .collect::<Result<Vec<_>, EngineError>>()?;

    let mut merged = BatchAcc::new(n_bins);
    for partial in partials {
        merged.merge(partial);
    }

    if n_sampled > 0 && n_sampled < n_rgs {
        let scale = n_rgs as f64 / n_sampled as f64;
        for c in merged.histogram_counts.iter_mut() {
            *c = (*c as f64 * scale).round() as u64;
        }
    }

    let distinct_count = merged.hll.estimate();
    let mean = if is_numeric && merged.count > 0 {
        Some(merged.sum / merged.count as f64)
    } else {
        None
    };

    let histogram = if is_numeric {
        if !has_range {
            if merged.count > 0 {
                Some(vec![HistogramBucket {
                    lo: histo_lo,
                    hi: histo_lo + 1.0,
                    count: merged.count,
                }])
            } else {
                Some(vec![])
            }
        } else {
            let buckets: Vec<HistogramBucket> = merged
                .histogram_counts
                .iter()
                .enumerate()
                .filter(|(_, &c)| c > 0)
                .map(|(i, &c)| HistogramBucket {
                    lo: histo_lo + i as f64 * bin_width,
                    hi: histo_lo + (i + 1) as f64 * bin_width,
                    count: c,
                })
                .collect();
            Some(buckets)
        }
    } else {
        None
    };

    let quantiles = if is_numeric {
        merged.reservoir.quantiles()
    } else {
        None
    };
    let top_values = if !is_numeric {
        merged.top_tracker.top_k(total_rows)
    } else {
        None
    };

    Ok((distinct_count, mean, histogram, quantiles, top_values))
}

fn compute_distinct_count_arrays(arrays: &[ArrayRef]) -> u64 {
    let mut hll = HyperLogLog::new();
    for arr in arrays {
        let str_col = match cast(arr.as_ref(), &ArrowDataType::Utf8) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Some(s) = str_col.as_any().downcast_ref::<StringArray>() {
            for i in 0..s.len() {
                if s.is_null(i) {
                    hll.add_null();
                } else {
                    hll.add_str(s.value(i));
                }
            }
        }
    }
    hll.estimate()
}

fn compute_min_max_arrays(
    arrays: &[ArrayRef],
    is_numeric: bool,
) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
    if is_numeric {
        let mut global_min: Option<f64> = None;
        let mut global_max: Option<f64> = None;
        for arr in arrays {
            let f64_col = match cast(arr.as_ref(), &ArrowDataType::Float64) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(f64_arr) = f64_col.as_any().downcast_ref::<Float64Array>() {
                if let Some(v) = arrow::compute::min(f64_arr) {
                    global_min = Some(global_min.map_or(v, |m| m.min(v)));
                }
                if let Some(v) = arrow::compute::max(f64_arr) {
                    global_max = Some(global_max.map_or(v, |m| m.max(v)));
                }
            }
        }
        let to_json = |f: f64| serde_json::Number::from_f64(f).map(serde_json::Value::Number);
        (global_min.and_then(to_json), global_max.and_then(to_json))
    } else {
        let mut min_str: Option<String> = None;
        let mut max_str: Option<String> = None;
        for arr in arrays {
            let str_col = match cast(arr.as_ref(), &ArrowDataType::Utf8) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(s) = str_col.as_any().downcast_ref::<StringArray>() {
                for i in 0..s.len() {
                    if s.is_null(i) {
                        continue;
                    }
                    let v = s.value(i);
                    if min_str.as_deref().map(|m| v < m).unwrap_or(true) {
                        min_str = Some(v.to_string());
                    }
                    if max_str.as_deref().map(|m| v > m).unwrap_or(true) {
                        max_str = Some(v.to_string());
                    }
                }
            }
        }
        (
            min_str.map(serde_json::Value::String),
            max_str.map(serde_json::Value::String),
        )
    }
}

fn compute_mean_arrays(arrays: &[ArrayRef]) -> Result<f64, EngineError> {
    let mut total_sum = 0.0f64;
    let mut total_count = 0u64;

    for arr in arrays {
        let f64_col = cast(arr.as_ref(), &ArrowDataType::Float64)?;
        let f64_arr = f64_col
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| EngineError::Query("float64 cast failed".into()))?;
        let non_null = (f64_arr.len() - f64_arr.null_count()) as u64;
        if let Some(s) = arrow::compute::sum(f64_arr) {
            total_sum += s;
            total_count += non_null;
        }
    }

    if total_count == 0 {
        Ok(0.0)
    } else {
        Ok(total_sum / total_count as f64)
    }
}

fn compute_histogram_arrays(
    arrays: &[ArrayRef],
    n_bins: usize,
) -> Result<Vec<HistogramBucket>, EngineError> {
    let f64_cols: Vec<arrow::array::ArrayRef> = arrays
        .iter()
        .filter_map(|arr| cast(arr.as_ref(), &ArrowDataType::Float64).ok())
        .collect();

    if f64_cols.is_empty() {
        return Ok(vec![]);
    }

    let mut global_lo: Option<f64> = None;
    let mut global_hi: Option<f64> = None;

    for col in &f64_cols {
        if let Some(f64_arr) = col.as_any().downcast_ref::<Float64Array>() {
            if let Some(v) = arrow::compute::min(f64_arr) {
                global_lo = Some(global_lo.map_or(v, |m: f64| m.min(v)));
            }
            if let Some(v) = arrow::compute::max(f64_arr) {
                global_hi = Some(global_hi.map_or(v, |m: f64| m.max(v)));
            }
        }
    }

    let (lo, hi) = match (global_lo, global_hi) {
        (Some(lo), Some(hi)) => (lo, hi),
        _ => return Ok(vec![]),
    };

    if lo >= hi {
        let total: u64 = f64_cols
            .iter()
            .filter_map(|c| c.as_any().downcast_ref::<Float64Array>())
            .map(|a| (a.len() - a.null_count()) as u64)
            .sum();
        return Ok(vec![HistogramBucket {
            lo,
            hi: lo + 1.0,
            count: total,
        }]);
    }

    let bin_width = (hi - lo) / n_bins as f64;
    let mut counts = vec![0u64; n_bins];

    for col in &f64_cols {
        if let Some(f64_arr) = col.as_any().downcast_ref::<Float64Array>() {
            for i in 0..f64_arr.len() {
                if !f64_arr.is_null(i) {
                    let v = f64_arr.value(i);
                    if v.is_finite() {
                        let bin = ((v - lo) / bin_width) as usize;
                        counts[bin.min(n_bins - 1)] += 1;
                    }
                }
            }
        }
    }

    Ok(counts
        .iter()
        .enumerate()
        .filter(|(_, &c)| c > 0)
        .map(|(i, &c)| HistogramBucket {
            lo: lo + i as f64 * bin_width,
            hi: lo + (i + 1) as f64 * bin_width,
            count: c,
        })
        .collect())
}

fn compute_quantiles_and_top_arrays(
    arrays: &[ArrayRef],
    is_numeric: bool,
    total_count: u64,
) -> (Option<Quantiles>, Option<Vec<TopValue>>) {
    if is_numeric {
        let mut reservoir = ReservoirSampler::new();
        for arr in arrays {
            let f64_col = match cast(arr.as_ref(), &ArrowDataType::Float64) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(f64_arr) = f64_col.as_any().downcast_ref::<Float64Array>() {
                for i in 0..f64_arr.len() {
                    if !f64_arr.is_null(i) {
                        let v = f64_arr.value(i);
                        if v.is_finite() {
                            reservoir.add(v);
                        }
                    }
                }
            }
        }
        (reservoir.quantiles(), None)
    } else {
        let mut tracker = TopValuesTracker::new();
        for arr in arrays {
            let str_col = match cast(arr.as_ref(), &ArrowDataType::Utf8) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(s) = str_col.as_any().downcast_ref::<StringArray>() {
                for i in 0..s.len() {
                    if !s.is_null(i) {
                        tracker.add(s.value(i));
                    }
                }
            }
        }
        (None, tracker.top_k(total_count))
    }
}

pub fn compute_correlations(meta: &SourceMeta) -> Result<CorrelationMatrix, EngineError> {
    let numeric_cols: Vec<&ColumnInfo> = meta
        .columns
        .iter()
        .filter(|c| is_numeric_type(&c.data_type))
        .collect();

    if numeric_cols.len() < 2 {
        return Ok(CorrelationMatrix {
            columns: numeric_cols.iter().map(|c| c.name.clone()).collect(),
            matrix: if numeric_cols.len() == 1 {
                vec![vec![Some(1.0)]]
            } else {
                vec![]
            },
        });
    }

    let n_cols = numeric_cols.len();
    let col_indices: Vec<usize> = numeric_cols.iter().map(|c| c.index).collect();

    if matches!(meta.format, tv_core::SourceFormat::Parquet)
        && !matches!(
            meta.kind,
            tv_core::SourceKind::S3
                | tv_core::SourceKind::Gcs
                | tv_core::SourceKind::AzureBlob
                | tv_core::SourceKind::Http
        )
    {
        let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
        return compute_correlations_streaming(path, &col_indices, &numeric_cols);
    }

    let batches = reader::read_parquet_full(&meta.uri, Some(&col_indices))?;
    compute_correlations_from_batches(&batches, n_cols, &numeric_cols)
}

#[allow(clippy::needless_range_loop)]
fn compute_correlations_streaming(
    path: &str,
    col_indices: &[usize],
    numeric_cols: &[&ColumnInfo],
) -> Result<CorrelationMatrix, EngineError> {
    let n_cols = numeric_cols.len();
    let n_pairs = n_cols * (n_cols - 1) / 2;
    let mut pairs: Vec<WelfordPair> = (0..n_pairs).map(|_| WelfordPair::new()).collect();

    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let parquet_schema = builder.parquet_schema();
    let mask = ProjectionMask::roots(parquet_schema, col_indices.iter().copied());
    let reader = builder.with_projection(mask).build()?;

    for batch_result in reader {
        let batch = batch_result.map_err(EngineError::Arrow)?;
        let n_rows = batch.num_rows();

        let cols: Vec<Vec<Option<f64>>> = (0..n_cols)
            .map(|out_idx| {
                let arr = batch.column(out_idx);
                let f64_col = cast(arr.as_ref(), &ArrowDataType::Float64).ok()?;
                let f64_arr = f64_col.as_any().downcast_ref::<Float64Array>()?.clone();
                Some(
                    (0..n_rows)
                        .map(|i| {
                            if f64_arr.is_null(i) {
                                None
                            } else {
                                let v = f64_arr.value(i);
                                if v.is_finite() {
                                    Some(v)
                                } else {
                                    None
                                }
                            }
                        })
                        .collect(),
                )
            })
            .map(|opt| opt.unwrap_or_else(|| vec![None; n_rows]))
            .collect();

        let mut pair_idx = 0;
        for i in 0..n_cols {
            for j in (i + 1)..n_cols {
                for (x_opt, y_opt) in cols[i].iter().zip(cols[j].iter()) {
                    if let (Some(x), Some(y)) = (*x_opt, *y_opt) {
                        pairs[pair_idx].update(x, y);
                    }
                }
                pair_idx += 1;
            }
        }
    }

    let mut matrix = vec![vec![None; n_cols]; n_cols];
    let mut pair_idx = 0;
    for i in 0..n_cols {
        matrix[i][i] = Some(1.0);
        for j in (i + 1)..n_cols {
            let r = pairs[pair_idx].correlation();
            matrix[i][j] = r;
            matrix[j][i] = r;
            pair_idx += 1;
        }
    }

    Ok(CorrelationMatrix {
        columns: numeric_cols.iter().map(|c| c.name.clone()).collect(),
        matrix,
    })
}

fn compute_correlations_from_batches(
    batches: &[arrow::record_batch::RecordBatch],
    n_cols: usize,
    numeric_cols: &[&ColumnInfo],
) -> Result<CorrelationMatrix, EngineError> {
    let mut columns_data: Vec<Vec<f64>> = vec![Vec::new(); n_cols];

    for batch in batches {
        for (out_idx, col_data) in columns_data.iter_mut().enumerate().take(n_cols) {
            let arr = batch.column(out_idx);
            let f64_col = cast(arr.as_ref(), &ArrowDataType::Float64)?;
            let f64_arr = f64_col
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| EngineError::Query("float64 cast failed".into()))?;
            for i in 0..f64_arr.len() {
                if !f64_arr.is_null(i) {
                    col_data.push(f64_arr.value(i));
                } else {
                    col_data.push(f64::NAN);
                }
            }
        }
    }

    let n_rows = columns_data[0].len();
    let mut matrix = vec![vec![None; n_cols]; n_cols];

    for i in 0..n_cols {
        for j in 0..n_cols {
            if i == j {
                matrix[i][j] = Some(1.0);
                continue;
            }
            let xi = &columns_data[i];
            let xj = &columns_data[j];

            let valid: Vec<(f64, f64)> = (0..n_rows)
                .filter(|&k| xi[k].is_finite() && xj[k].is_finite())
                .map(|k| (xi[k], xj[k]))
                .collect();

            if valid.len() < 2 {
                matrix[i][j] = None;
                continue;
            }

            let n = valid.len() as f64;
            let mean_i = valid.iter().map(|(a, _)| a).sum::<f64>() / n;
            let mean_j = valid.iter().map(|(_, b)| b).sum::<f64>() / n;
            let cov: f64 = valid.iter().map(|(a, b)| (a - mean_i) * (b - mean_j)).sum();
            let std_i: f64 = valid
                .iter()
                .map(|(a, _)| (a - mean_i).powi(2))
                .sum::<f64>()
                .sqrt();
            let std_j: f64 = valid
                .iter()
                .map(|(_, b)| (b - mean_j).powi(2))
                .sum::<f64>()
                .sqrt();

            matrix[i][j] = if std_i > 0.0 && std_j > 0.0 {
                Some(cov / (std_i * std_j))
            } else {
                None
            };
        }
    }

    Ok(CorrelationMatrix {
        columns: numeric_cols.iter().map(|c| c.name.clone()).collect(),
        matrix,
    })
}

pub fn is_numeric_type(dtype: &str) -> bool {
    let lower = dtype.to_lowercase();
    lower.contains("int")
        || lower.contains("float")
        || lower.contains("double")
        || lower.contains("decimal")
        || lower.contains("numeric")
        || lower.contains("real")
        || lower.contains("uint")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{int_string_batch, write_test_parquet};
    use tempfile::TempDir;

    fn make_dir() -> TempDir {
        tempfile::TempDir::new().unwrap()
    }

    #[test]
    fn hll_exact_small_cardinality() {
        let mut hll = HyperLogLog::new();
        for i in 0..10u64 {
            hll.add_hash(mix64(i));
        }
        let est = hll.estimate();
        assert!(est >= 8 && est <= 12, "estimate was {est}");
    }

    #[test]
    fn hll_large_cardinality_within_5pct() {
        let mut hll = HyperLogLog::new();
        let n = 10_000u64;
        for i in 0..n {
            hll.add_hash(mix64(i.wrapping_mul(0x9e3779b97f4a7c15u64.wrapping_add(i))));
        }
        let est = hll.estimate();
        let error = (est as f64 - n as f64).abs() / n as f64;
        assert!(error < 0.05, "error {error:.3} >= 5%");
    }

    #[test]
    fn hll_merge_preserves_estimate() {
        let mut hll1 = HyperLogLog::new();
        let mut hll2 = HyperLogLog::new();
        for i in 0..500u64 {
            hll1.add_hash(mix64(i));
        }
        for i in 500..1000u64 {
            hll2.add_hash(mix64(i));
        }
        hll1.merge(&hll2);
        let est = hll1.estimate();
        assert!(est >= 800 && est <= 1200, "estimate was {est}");
    }

    #[test]
    fn hll_all_nulls() {
        let mut hll = HyperLogLog::new();
        for _ in 0..100 {
            hll.add_null();
        }
        let est = hll.estimate();
        assert_eq!(est, 1);
    }

    #[test]
    fn welford_perfect_correlation() {
        let mut wp = WelfordPair::new();
        for i in 0..100 {
            let v = i as f64;
            wp.update(v, v * 2.0);
        }
        let corr = wp.correlation().unwrap();
        assert!((corr - 1.0).abs() < 1e-9);
    }

    #[test]
    fn welford_no_correlation() {
        let mut wp = WelfordPair::new();
        wp.update(1.0, 3.0);
        wp.update(2.0, 3.0);
        wp.update(3.0, 3.0);
        assert!(wp.correlation().is_none());
    }

    #[test]
    fn welford_single_pair_returns_none() {
        let mut wp = WelfordPair::new();
        wp.update(1.0, 2.0);
        assert!(wp.correlation().is_none());
    }

    #[test]
    fn reservoir_small_input() {
        let mut rs = ReservoirSampler::new();
        for i in 0..5 {
            rs.add(i as f64);
        }
        let q = rs.quantiles().unwrap();
        assert!(q.p50 >= 0.0 && q.p50 <= 4.0);
    }

    #[test]
    fn reservoir_quantiles_sorted() {
        let mut rs = ReservoirSampler::new();
        for i in (0..100).rev() {
            rs.add(i as f64);
        }
        let q = rs.quantiles().unwrap();
        assert!(q.p1 <= q.p5);
        assert!(q.p5 <= q.p25);
        assert!(q.p25 <= q.p50);
        assert!(q.p50 <= q.p75);
        assert!(q.p75 <= q.p95);
        assert!(q.p95 <= q.p99);
    }

    #[test]
    fn top_values_low_cardinality() {
        let mut tracker = TopValuesTracker::new();
        for _ in 0..10 {
            tracker.add("apple");
        }
        for _ in 0..5 {
            tracker.add("banana");
        }
        let top = tracker.top_k(15).unwrap();
        assert!(!top.is_empty());
        assert_eq!(top[0].value.as_str().unwrap(), "apple");
        assert_eq!(top[0].count, 10);
    }

    #[test]
    fn top_values_overflow() {
        let mut tracker = TopValuesTracker::new();
        for i in 0..TOP_VALUES_MAX_DISTINCT + 10 {
            tracker.add(&format!("val_{i}"));
        }
        assert!(tracker.top_k(TOP_VALUES_MAX_DISTINCT as u64 + 10).is_none());
    }

    #[test]
    fn classify_cardinality_constant() {
        assert_eq!(classify_cardinality(1, 100), CardinalityCategory::Constant);
    }

    #[test]
    fn classify_cardinality_categories() {
        assert_eq!(classify_cardinality(2, 100), CardinalityCategory::Binary);
        assert_eq!(
            classify_cardinality(10, 100),
            CardinalityCategory::LowCardinality
        );
        assert_eq!(classify_cardinality(100, 100), CardinalityCategory::Unique);
        assert_eq!(classify_cardinality(0, 100), CardinalityCategory::Unknown);
        assert_eq!(classify_cardinality(1, 0), CardinalityCategory::Unknown);
    }

    #[test]
    fn compute_column_stats_basic() {
        use crate::reader::parquet_schema_and_rows;
        use tv_core::{ColumnInfo, SourceFormat, SourceKind, SourceMeta};

        let dir = make_dir();
        let batch = int_string_batch(100);
        let path = write_test_parquet(&dir, "test.parquet", &[batch]);
        let path_str = path.to_str().unwrap();
        let (schema, n_rows) = parquet_schema_and_rows(path_str).unwrap();

        let meta = SourceMeta {
            id: "test".into(),
            name: "test".into(),
            uri: path_str.into(),
            files: vec![path_str.into()],
            format: SourceFormat::Parquet,
            kind: SourceKind::LocalFile,
            n_rows,
            n_cols: schema.fields().len(),
            columns: schema
                .fields()
                .iter()
                .enumerate()
                .map(|(i, f)| ColumnInfo {
                    index: i,
                    name: f.name().clone(),
                    data_type: format!("{:?}", f.data_type()),
                    nullable: f.is_nullable(),
                })
                .collect(),
            quick_stats: vec![],
            tile_rows: 256,
            file_mtime_secs: 0,
            file_size_bytes: 0,
            recommendations: vec![],
            pre_sorted_by: None,
        };

        let stats = compute_column_stats(&meta, 0, DEFAULT_HISTOGRAM_BINS).unwrap();
        assert_eq!(stats.count, 100);
        assert_eq!(stats.null_count, 0);
        assert!(stats.distinct_count.is_some());
    }
}
