use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, Float64Array, Int64Array, StringArray};
use arrow::compute::cast;
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use arrow::row::{RowConverter, SortField};
use tv_core::{AggExpr, SortKey};

use crate::error::EngineError;
use crate::executor::apply_sort;
use crate::spill::{SpillWriter, SpilledRun};

fn cell_as_string(batch: &RecordBatch, col_idx: usize, row: usize) -> Option<String> {
    let col = batch.column(col_idx);
    if col.is_null(row) {
        return None;
    }
    cast(col.as_ref(), &ArrowDataType::Utf8).ok().and_then(|c| {
        c.as_any()
            .downcast_ref::<StringArray>()
            .map(|s| s.value(row).to_string())
    })
}

fn cell_as_f64(batch: &RecordBatch, col_idx: usize, row: usize) -> Option<f64> {
    let col = batch.column(col_idx);
    if col.is_null(row) {
        return None;
    }
    cast(col.as_ref(), &ArrowDataType::Float64)
        .ok()
        .and_then(|c| {
            c.as_any()
                .downcast_ref::<Float64Array>()
                .map(|a| a.value(row))
        })
}

#[derive(Clone)]
enum PartialAgg {
    Count(i64),
    CountDistinct(Vec<String>),
    Sum(Option<f64>),
    Min(Option<f64>),
    Max(Option<f64>),
    Mean { n: u64, sum: f64 },
    StdDev { n: u64, sum: f64, m2: f64 },
    Median(Vec<f64>),
    Percentile { vals: Vec<f64>, p: f64 },
}

impl PartialAgg {
    fn new_for(agg: &AggExpr) -> Self {
        match agg {
            AggExpr::Count { .. } => PartialAgg::Count(0),
            AggExpr::CountDistinct { .. } => PartialAgg::CountDistinct(Vec::new()),
            AggExpr::Sum { .. } => PartialAgg::Sum(None),
            AggExpr::Min { .. } => PartialAgg::Min(None),
            AggExpr::Max { .. } => PartialAgg::Max(None),
            AggExpr::Mean { .. } => PartialAgg::Mean { n: 0, sum: 0.0 },
            AggExpr::StdDev { .. } => PartialAgg::StdDev {
                n: 0,
                sum: 0.0,
                m2: 0.0,
            },
            AggExpr::Median { .. } => PartialAgg::Median(Vec::new()),
            AggExpr::Percentile { p, .. } => PartialAgg::Percentile {
                vals: Vec::new(),
                p: *p,
            },
        }
    }

    fn update(&mut self, fval: Option<f64>, sval: Option<&str>) {
        match self {
            PartialAgg::Count(n) => *n += 1,
            PartialAgg::CountDistinct(seen) => {
                if let Some(s) = sval {
                    let owned = s.to_string();
                    if !seen.contains(&owned) {
                        seen.push(owned);
                    }
                }
            }
            PartialAgg::Sum(acc) => {
                if let Some(v) = fval {
                    *acc = Some(acc.unwrap_or(0.0) + v);
                }
            }
            PartialAgg::Min(acc) => {
                if let Some(v) = fval {
                    *acc = Some(acc.map(|a: f64| a.min(v)).unwrap_or(v));
                }
            }
            PartialAgg::Max(acc) => {
                if let Some(v) = fval {
                    *acc = Some(acc.map(|a: f64| a.max(v)).unwrap_or(v));
                }
            }
            PartialAgg::Mean { n, sum } => {
                if let Some(v) = fval {
                    *n += 1;
                    *sum += v;
                }
            }
            PartialAgg::StdDev { n, sum, m2 } => {
                if let Some(v) = fval {
                    *n += 1;
                    let old_mean = if *n > 1 { *sum / (*n - 1) as f64 } else { 0.0 };
                    *sum += v;
                    let new_mean = *sum / *n as f64;
                    *m2 += (v - old_mean) * (v - new_mean);
                }
            }
            PartialAgg::Median(vals) => {
                if let Some(v) = fval {
                    vals.push(v);
                }
            }
            PartialAgg::Percentile { vals, .. } => {
                if let Some(v) = fval {
                    vals.push(v);
                }
            }
        }
    }

    fn finalize_i64(&self) -> Option<i64> {
        match self {
            PartialAgg::Count(n) => Some(*n),
            PartialAgg::CountDistinct(seen) => Some(seen.len() as i64),
            _ => None,
        }
    }

    fn finalize_f64(&self) -> Option<f64> {
        match self {
            PartialAgg::Sum(v) => *v,
            PartialAgg::Min(v) => *v,
            PartialAgg::Max(v) => *v,
            PartialAgg::Mean { n, sum } => {
                if *n == 0 {
                    None
                } else {
                    Some(sum / *n as f64)
                }
            }
            PartialAgg::StdDev { n, m2, .. } => {
                if *n < 2 {
                    None
                } else {
                    Some((m2 / (*n - 1) as f64).sqrt())
                }
            }
            PartialAgg::Median(vals) => {
                if vals.is_empty() {
                    return None;
                }
                let mut s = vals.clone();
                s.sort_by(|a, b| a.total_cmp(b));
                let mid = s.len() / 2;
                if s.len() % 2 == 0 {
                    Some((s[mid - 1] + s[mid]) / 2.0)
                } else {
                    Some(s[mid])
                }
            }
            PartialAgg::Percentile { vals, p } => {
                if vals.is_empty() {
                    return None;
                }
                let mut s = vals.clone();
                s.sort_by(|a, b| a.total_cmp(b));
                let idx = ((p / 100.0) * (s.len() - 1) as f64).round() as usize;
                Some(s[idx.min(s.len() - 1)])
            }
            _ => None,
        }
    }
}

struct GroupEntry {
    key_str_values: Vec<Option<String>>,
    accs: Vec<PartialAgg>,
}

pub struct StreamingAggregator {
    pub key_columns: Vec<String>,
    pub aggs: Vec<AggExpr>,
    pub output_schema: SchemaRef,
}

impl StreamingAggregator {
    pub fn new(
        key_columns: Vec<String>,
        aggs: Vec<AggExpr>,
        input_schema: &SchemaRef,
    ) -> Result<Self, EngineError> {
        let output_schema = build_output_schema(input_schema, &key_columns, &aggs)?;
        Ok(Self {
            key_columns,
            aggs,
            output_schema,
        })
    }

    pub fn execute(
        &self,
        stream: impl Iterator<Item = Result<RecordBatch, EngineError>>,
        writer: &mut SpillWriter,
    ) -> Result<(SpilledRun, u64), EngineError> {
        let mut groups: HashMap<Vec<u8>, GroupEntry> = HashMap::new();
        let mut row_converter: Option<RowConverter> = None;

        for batch_result in stream {
            let batch = batch_result?;
            if batch.num_rows() == 0 {
                continue;
            }
            let key_indices: Vec<usize> = self
                .key_columns
                .iter()
                .filter_map(|name| batch.schema().index_of(name).ok())
                .collect();

            let encoded_rows = if key_indices.is_empty() {
                None
            } else {
                if row_converter.is_none() {
                    let sort_fields: Vec<SortField> = key_indices
                        .iter()
                        .map(|&ki| SortField::new(batch.schema().field(ki).data_type().clone()))
                        .collect();
                    row_converter =
                        Some(RowConverter::new(sort_fields).map_err(EngineError::Arrow)?);
                }
                let converter = row_converter.as_ref().unwrap();
                let key_cols: Vec<ArrayRef> = key_indices
                    .iter()
                    .map(|&ki| batch.column(ki).clone())
                    .collect();
                Some(
                    converter
                        .convert_columns(&key_cols)
                        .map_err(EngineError::Arrow)?,
                )
            };

            for row in 0..batch.num_rows() {
                let key: Vec<u8> = match &encoded_rows {
                    Some(rows) => rows.row(row).as_ref().to_vec(),
                    None => vec![],
                };
                let entry = groups.entry(key).or_insert_with(|| {
                    let key_str_values = key_indices
                        .iter()
                        .map(|&ki| cell_as_string(&batch, ki, row))
                        .collect();
                    GroupEntry {
                        key_str_values,
                        accs: self.aggs.iter().map(PartialAgg::new_for).collect(),
                    }
                });

                for (acc, agg) in entry.accs.iter_mut().zip(&self.aggs) {
                    let col_name = agg_col_name(agg);
                    let (fval, sval) = match col_name {
                        None => (Some(1.0f64), None),
                        Some(name) => match batch.schema().index_of(name).ok() {
                            None => (None, None),
                            Some(ci) => (
                                cell_as_f64(&batch, ci, row),
                                cell_as_string(&batch, ci, row),
                            ),
                        },
                    };
                    acc.update(fval, sval.as_deref());
                }
            }
        }

        let batches = self.finalize_groups(groups)?;
        let total_rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
        let run = writer.write_run(&batches)?;
        Ok((run, total_rows))
    }

    fn finalize_groups(
        &self,
        groups: HashMap<Vec<u8>, GroupEntry>,
    ) -> Result<Vec<RecordBatch>, EngineError> {
        if groups.is_empty() {
            let empty_cols: Vec<ArrayRef> = self
                .output_schema
                .fields()
                .iter()
                .map(|f| arrow::array::new_null_array(f.data_type(), 0))
                .collect();
            return Ok(vec![RecordBatch::try_new(
                self.output_schema.clone(),
                empty_cols,
            )?]);
        }

        let sort_keys: Vec<SortKey> = self
            .key_columns
            .iter()
            .map(|k| SortKey {
                column: k.clone(),
                descending: false,
                nulls_last: true,
            })
            .collect();

        let entries: Vec<GroupEntry> = groups.into_values().collect();
        let n = entries.len();
        let n_keys = self.key_columns.len();

        let mut key_str_cols: Vec<Vec<Option<String>>> = vec![Vec::with_capacity(n); n_keys];
        let mut agg_results_i64: Vec<Vec<Option<i64>>> =
            vec![Vec::with_capacity(n); self.aggs.len()];
        let mut agg_results_f64: Vec<Vec<Option<f64>>> =
            vec![Vec::with_capacity(n); self.aggs.len()];
        let mut agg_is_i64: Vec<bool> = Vec::with_capacity(self.aggs.len());
        for agg in &self.aggs {
            agg_is_i64.push(matches!(
                agg,
                AggExpr::Count { .. } | AggExpr::CountDistinct { .. }
            ));
        }

        for entry in &entries {
            for (ki, kv) in entry.key_str_values.iter().enumerate() {
                key_str_cols[ki].push(kv.clone());
            }
            for (ai, acc) in entry.accs.iter().enumerate() {
                if agg_is_i64[ai] {
                    agg_results_i64[ai].push(acc.finalize_i64());
                } else {
                    agg_results_f64[ai].push(acc.finalize_f64());
                }
            }
        }

        let mut cols: Vec<ArrayRef> = Vec::with_capacity(n_keys + self.aggs.len());

        for (ki, col_name) in self.key_columns.iter().enumerate() {
            let string_arr: ArrayRef = Arc::new(StringArray::from(key_str_cols[ki].clone()));
            let target_type = self.output_schema.field(ki).data_type();
            let arr = if *target_type == ArrowDataType::Utf8 {
                string_arr
            } else {
                cast(string_arr.as_ref(), target_type)?
            };
            cols.push(arr);
            let _ = col_name;
        }

        for (ai, _agg) in self.aggs.iter().enumerate() {
            let arr: ArrayRef = if agg_is_i64[ai] {
                Arc::new(Int64Array::from(agg_results_i64[ai].clone()))
            } else {
                Arc::new(Float64Array::from(agg_results_f64[ai].clone()))
            };
            cols.push(arr);
        }

        let batch = RecordBatch::try_new(self.output_schema.clone(), cols)?;
        if !sort_keys.is_empty() {
            apply_sort(vec![batch], &sort_keys)
        } else {
            Ok(vec![batch])
        }
    }
}

fn agg_col_name(agg: &AggExpr) -> Option<&str> {
    match agg {
        AggExpr::Count { .. } => None,
        AggExpr::CountDistinct { column, .. } => Some(column.as_str()),
        AggExpr::Sum { column, .. } => Some(column.as_str()),
        AggExpr::Min { column, .. } => Some(column.as_str()),
        AggExpr::Max { column, .. } => Some(column.as_str()),
        AggExpr::Mean { column, .. } => Some(column.as_str()),
        AggExpr::StdDev { column, .. } => Some(column.as_str()),
        AggExpr::Median { column, .. } => Some(column.as_str()),
        AggExpr::Percentile { column, .. } => Some(column.as_str()),
    }
}

pub struct SampleAggregator {
    pub inner: StreamingAggregator,
    pub sample_rows: u64,
    pub seed: Option<u64>,
}

impl SampleAggregator {
    pub fn new(
        key_columns: Vec<String>,
        aggs: Vec<AggExpr>,
        input_schema: &SchemaRef,
        sample_rows: u64,
        seed: Option<u64>,
    ) -> Result<Self, EngineError> {
        let inner = StreamingAggregator::new(key_columns, aggs, input_schema)?;
        Ok(Self {
            inner,
            sample_rows,
            seed,
        })
    }

    pub fn execute_sampled(
        &self,
        stream: impl Iterator<Item = Result<arrow::record_batch::RecordBatch, EngineError>>,
        writer: &mut crate::spill::SpillWriter,
    ) -> Result<(crate::spill::SpilledRun, u64), EngineError> {
        let mut rng = self.seed.unwrap_or(42);
        let mut reservoir: Vec<arrow::record_batch::RecordBatch> = Vec::new();
        let mut total_seen = 0u64;
        let k = self.sample_rows as usize;

        for batch_result in stream {
            let batch = batch_result?;
            if batch.num_rows() == 0 {
                continue;
            }

            for row_idx in 0..batch.num_rows() {
                total_seen += 1;
                let row_batch = batch.slice(row_idx, 1);
                if reservoir.len() < k {
                    reservoir.push(row_batch);
                } else {
                    rng = mix64(rng);
                    let j = (rng as usize) % total_seen as usize;
                    if j < k {
                        reservoir[j] = row_batch;
                    }
                }
            }
        }

        self.inner.execute(reservoir.into_iter().map(Ok), writer)
    }
}

fn mix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

fn build_output_schema(
    input_schema: &SchemaRef,
    key_columns: &[String],
    aggs: &[AggExpr],
) -> Result<SchemaRef, EngineError> {
    let mut fields: Vec<Arc<Field>> = Vec::new();
    for key in key_columns {
        let idx = input_schema
            .index_of(key)
            .map_err(|_| EngineError::Query(format!("group-by key column not found: {key}")))?;
        fields.push(Arc::new(input_schema.field(idx).clone()));
    }
    for agg in aggs {
        let alias = tv_core::agg_alias(agg).to_string();
        let dt = match agg {
            AggExpr::Count { .. } | AggExpr::CountDistinct { .. } => ArrowDataType::Int64,
            _ => ArrowDataType::Float64,
        };
        fields.push(Arc::new(Field::new(&alias, dt, true)));
    }
    Ok(Arc::new(Schema::new(fields)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spill::SpillWriter;
    use crate::test_helpers::{int_string_batch, people_batches};
    use arrow::array::{Float64Array, Int64Array};
    use tempfile::TempDir;
    use tv_core::AggExpr;

    fn collect_i64_column(batches: &[RecordBatch], col_name: &str) -> Vec<i64> {
        batches
            .iter()
            .flat_map(|b| {
                let idx = b.schema().index_of(col_name).unwrap();
                b.column(idx)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values()
                    .to_vec()
            })
            .collect()
    }

    fn collect_f64_column(batches: &[RecordBatch], col_name: &str) -> Vec<f64> {
        batches
            .iter()
            .flat_map(|b| {
                let idx = b.schema().index_of(col_name).unwrap();
                b.column(idx)
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap()
                    .values()
                    .to_vec()
            })
            .collect()
    }

    fn read_run_batches(run: &crate::spill::SpilledRun) -> Vec<RecordBatch> {
        crate::spill::SpillReader::open(&run.path)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn streaming_count_all() {
        let tmp = TempDir::new().unwrap();
        let batch = int_string_batch(10);
        let schema = batch.schema();
        let aggs = vec![AggExpr::Count {
            alias: "cnt".to_string(),
        }];
        let aggregator = StreamingAggregator::new(vec![], aggs, &schema).unwrap();
        let mut writer =
            SpillWriter::new(tmp.path().to_path_buf(), aggregator.output_schema.clone());
        let (run, _) = aggregator
            .execute(vec![Ok(batch)].into_iter(), &mut writer)
            .unwrap();
        let batches = read_run_batches(&run);
        let counts = collect_i64_column(&batches, "cnt");
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0], 10);
    }

    #[test]
    fn streaming_count_distinct() {
        let tmp = TempDir::new().unwrap();
        let people = people_batches();
        let schema = people[0].schema();
        let aggs = vec![AggExpr::CountDistinct {
            column: "department".to_string(),
            alias: "dept_count".to_string(),
        }];
        let aggregator = StreamingAggregator::new(vec![], aggs, &schema).unwrap();
        let mut writer =
            SpillWriter::new(tmp.path().to_path_buf(), aggregator.output_schema.clone());
        let (run, _) = aggregator
            .execute(people.into_iter().map(Ok), &mut writer)
            .unwrap();
        let batches = read_run_batches(&run);
        let counts = collect_i64_column(&batches, "dept_count");
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0], 4);
    }

    #[test]
    fn streaming_sum() {
        let tmp = TempDir::new().unwrap();
        let batch = int_string_batch(10);
        let schema = batch.schema();
        let aggs = vec![AggExpr::Sum {
            column: "id".to_string(),
            alias: "id_sum".to_string(),
        }];
        let aggregator = StreamingAggregator::new(vec![], aggs, &schema).unwrap();
        let mut writer =
            SpillWriter::new(tmp.path().to_path_buf(), aggregator.output_schema.clone());
        let (run, _) = aggregator
            .execute(vec![Ok(batch)].into_iter(), &mut writer)
            .unwrap();
        let batches = read_run_batches(&run);
        let sums = collect_f64_column(&batches, "id_sum");
        assert_eq!(sums.len(), 1);
        assert!((sums[0] - 45.0).abs() < 1e-9);
    }

    #[test]
    fn streaming_min_max() {
        let tmp = TempDir::new().unwrap();
        let batch = int_string_batch(10);
        let schema = batch.schema();
        let aggs = vec![
            AggExpr::Min {
                column: "id".to_string(),
                alias: "id_min".to_string(),
            },
            AggExpr::Max {
                column: "id".to_string(),
                alias: "id_max".to_string(),
            },
        ];
        let aggregator = StreamingAggregator::new(vec![], aggs, &schema).unwrap();
        let mut writer =
            SpillWriter::new(tmp.path().to_path_buf(), aggregator.output_schema.clone());
        let (run, _) = aggregator
            .execute(vec![Ok(batch)].into_iter(), &mut writer)
            .unwrap();
        let batches = read_run_batches(&run);
        let mins = collect_f64_column(&batches, "id_min");
        let maxs = collect_f64_column(&batches, "id_max");
        assert!((mins[0] - 0.0).abs() < 1e-9);
        assert!((maxs[0] - 9.0).abs() < 1e-9);
    }

    #[test]
    fn streaming_mean() {
        let tmp = TempDir::new().unwrap();
        let batch = int_string_batch(10);
        let schema = batch.schema();
        let aggs = vec![AggExpr::Mean {
            column: "id".to_string(),
            alias: "id_mean".to_string(),
        }];
        let aggregator = StreamingAggregator::new(vec![], aggs, &schema).unwrap();
        let mut writer =
            SpillWriter::new(tmp.path().to_path_buf(), aggregator.output_schema.clone());
        let (run, _) = aggregator
            .execute(vec![Ok(batch)].into_iter(), &mut writer)
            .unwrap();
        let batches = read_run_batches(&run);
        let means = collect_f64_column(&batches, "id_mean");
        assert!((means[0] - 4.5).abs() < 1e-9);
    }

    #[test]
    fn streaming_group_by_multiple_keys() {
        let tmp = TempDir::new().unwrap();
        let people = people_batches();
        let schema = people[0].schema();
        let aggs = vec![AggExpr::Count {
            alias: "cnt".to_string(),
        }];
        let aggregator =
            StreamingAggregator::new(vec!["department".to_string()], aggs, &schema).unwrap();
        let mut writer =
            SpillWriter::new(tmp.path().to_path_buf(), aggregator.output_schema.clone());
        let (run, group_count) = aggregator
            .execute(people.into_iter().map(Ok), &mut writer)
            .unwrap();
        assert_eq!(group_count, 4);
        let batches = read_run_batches(&run);
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 4);
        let counts = collect_i64_column(&batches, "cnt");
        assert_eq!(counts.iter().sum::<i64>(), 200);
    }
}
