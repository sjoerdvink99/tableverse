use std::collections::HashSet;
use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int32Array, Int64Array, StringArray, UInt32Array,
};
use arrow::compute::{
    and, cast, concat_batches, filter_record_batch, is_not_null, is_null, lexsort_to_indices, not,
    or, take, SortColumn, SortOptions,
};
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use tv_core::{
    AggExpr, BinOp, DataType, FilterExpr, Literal, Predicate, ScalarExpr, SortKey, ViewOp,
};

use crate::error::EngineError;

#[derive(Clone, Copy)]
enum CmpOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

pub fn execute_pipeline(
    batches: Vec<RecordBatch>,
    ops: &[ViewOp],
) -> Result<Vec<RecordBatch>, EngineError> {
    if batches.is_empty() {
        return Ok(batches);
    }
    let normalized = tv_core::normalize_ops(ops);
    let mut current = batches;
    for op in &normalized {
        if current.is_empty() {
            break;
        }
        current = apply_op(current, op)?;
    }
    Ok(current)
}

pub fn execute_pipeline_skip_filter(
    batches: Vec<RecordBatch>,
    ops: &[ViewOp],
) -> Result<Vec<RecordBatch>, EngineError> {
    if batches.is_empty() {
        return Ok(batches);
    }
    let normalized = tv_core::normalize_ops(ops);
    let mut current = batches;
    for op in &normalized {
        if current.is_empty() {
            break;
        }
        if matches!(op, ViewOp::Filter { .. }) {
            continue;
        }
        current = apply_op(current, op)?;
    }
    Ok(current)
}

fn apply_op(batches: Vec<RecordBatch>, op: &ViewOp) -> Result<Vec<RecordBatch>, EngineError> {
    match op {
        ViewOp::Filter { predicate } => apply_filter(batches, predicate),
        ViewOp::Select { columns } => apply_select(batches, columns),
        ViewOp::Drop { columns } => apply_drop(batches, columns),
        ViewOp::Sort { keys } => apply_sort(batches, keys),
        ViewOp::Derive { name, expr } => apply_derive(batches, name, expr),
        ViewOp::Deduplicate { columns } => apply_deduplicate(batches, columns),
        ViewOp::Sample { n, seed, .. } => apply_sample(batches, *n, *seed),
        ViewOp::GroupBy { keys, aggs } => apply_group_by(batches, keys, aggs),
        ViewOp::Rename { mappings } => apply_rename(batches, mappings),
        ViewOp::Limit { n } => apply_limit(batches, *n),
        ViewOp::TopK { .. } | ViewOp::Approximate { .. } => Ok(batches),
    }
}

fn get_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a ArrayRef, EngineError> {
    let idx = batch
        .schema()
        .index_of(name)
        .map_err(|_| EngineError::Query(format!("column not found: {name}")))?;
    Ok(batch.column(idx))
}

fn compare_lit(col: &ArrayRef, value: &Literal, op: CmpOp) -> Result<BooleanArray, EngineError> {
    let n = col.len();

    match value {
        Literal::Null => Ok(BooleanArray::from(vec![false; n])),
        Literal::Bool(b) => {
            let casted = cast(col.as_ref(), &ArrowDataType::Boolean)?;
            let arr = casted
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(|| EngineError::InvalidCast {
                    col: "bool".into(),
                    expected: "BooleanArray",
                })?;
            let result: Vec<Option<bool>> = (0..n)
                .map(|i| {
                    if arr.is_null(i) {
                        return None;
                    }
                    let v = arr.value(i);
                    Some(match op {
                        CmpOp::Eq => v == *b,
                        CmpOp::Ne => v != *b,
                        CmpOp::Gt => v & !b,
                        CmpOp::Gte => v >= *b,
                        CmpOp::Lt => !v & *b,
                        CmpOp::Lte => v <= *b,
                    })
                })
                .collect();
            Ok(BooleanArray::from(result))
        }
        Literal::Int(i) => {
            let fval = *i as f64;
            compare_float(col, fval, op)
        }
        Literal::Float(f) => compare_float(col, *f, op),
        Literal::Text(s) => {
            use arrow::array::Scalar;
            use arrow::compute::kernels::cmp as arrow_cmp;
            let casted = cast(col.as_ref(), &ArrowDataType::Utf8)?;
            let scalar = Scalar::new(StringArray::from(vec![s.as_str()]));
            Ok(match op {
                CmpOp::Eq => arrow_cmp::eq(&casted.as_ref(), &scalar)?,
                CmpOp::Ne => arrow_cmp::neq(&casted.as_ref(), &scalar)?,
                CmpOp::Gt => arrow_cmp::gt(&casted.as_ref(), &scalar)?,
                CmpOp::Gte => arrow_cmp::gt_eq(&casted.as_ref(), &scalar)?,
                CmpOp::Lt => arrow_cmp::lt(&casted.as_ref(), &scalar)?,
                CmpOp::Lte => arrow_cmp::lt_eq(&casted.as_ref(), &scalar)?,
            })
        }
    }
}

fn compare_float(col: &ArrayRef, fval: f64, op: CmpOp) -> Result<BooleanArray, EngineError> {
    use arrow::array::Scalar;
    use arrow::compute::kernels::cmp as arrow_cmp;
    let casted = cast(col.as_ref(), &ArrowDataType::Float64)?;
    let scalar = Scalar::new(Float64Array::from(vec![fval]));
    Ok(match op {
        CmpOp::Eq => arrow_cmp::eq(&casted.as_ref(), &scalar)?,
        CmpOp::Ne => arrow_cmp::neq(&casted.as_ref(), &scalar)?,
        CmpOp::Gt => arrow_cmp::gt(&casted.as_ref(), &scalar)?,
        CmpOp::Gte => arrow_cmp::gt_eq(&casted.as_ref(), &scalar)?,
        CmpOp::Lt => arrow_cmp::lt(&casted.as_ref(), &scalar)?,
        CmpOp::Lte => arrow_cmp::lt_eq(&casted.as_ref(), &scalar)?,
    })
}

fn compare_json(
    col: &ArrayRef,
    value: &serde_json::Value,
    op: CmpOp,
) -> Result<BooleanArray, EngineError> {
    let n = col.len();
    match value {
        serde_json::Value::Null => Ok(BooleanArray::from(vec![false; n])),
        serde_json::Value::Bool(b) => compare_lit(col, &Literal::Bool(*b), op),
        serde_json::Value::Number(num) => {
            let fval = num.as_f64().unwrap_or(0.0);
            compare_float(col, fval, op)
        }
        serde_json::Value::String(s) => compare_lit(col, &Literal::Text(s.clone()), op),
        _ => Ok(BooleanArray::from(vec![false; n])),
    }
}

fn string_match(
    batch: &RecordBatch,
    column: &str,
    pred: impl Fn(&str) -> bool,
) -> Result<BooleanArray, EngineError> {
    let col = get_column(batch, column)?;
    let str_col = cast(col.as_ref(), &ArrowDataType::Utf8)?;
    let str_arr = str_col
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| EngineError::Query("cast to utf8 failed".into()))?;
    let mask: Vec<bool> = (0..str_arr.len())
        .map(|i| !str_arr.is_null(i) && pred(str_arr.value(i)))
        .collect();
    Ok(BooleanArray::from(mask))
}

pub fn predicate_to_bool_array(
    batch: &RecordBatch,
    pred: &Predicate,
) -> Result<BooleanArray, EngineError> {
    let n = batch.num_rows();
    match pred {
        Predicate::IsNull { column } => {
            let col = get_column(batch, column)?;
            Ok(is_null(col.as_ref())?)
        }
        Predicate::IsNotNull { column } => {
            let col = get_column(batch, column)?;
            Ok(is_not_null(col.as_ref())?)
        }
        Predicate::Eq { column, value } => {
            let col = get_column(batch, column)?;
            compare_lit(col, value, CmpOp::Eq)
        }
        Predicate::Ne { column, value } => {
            let col = get_column(batch, column)?;
            compare_lit(col, value, CmpOp::Ne)
        }
        Predicate::Gt { column, value } => {
            let col = get_column(batch, column)?;
            compare_lit(col, value, CmpOp::Gt)
        }
        Predicate::Gte { column, value } => {
            let col = get_column(batch, column)?;
            compare_lit(col, value, CmpOp::Gte)
        }
        Predicate::Lt { column, value } => {
            let col = get_column(batch, column)?;
            compare_lit(col, value, CmpOp::Lt)
        }
        Predicate::Lte { column, value } => {
            let col = get_column(batch, column)?;
            compare_lit(col, value, CmpOp::Lte)
        }
        Predicate::Between { column, lo, hi } => {
            let col = get_column(batch, column)?;
            let lo_mask = compare_lit(col, lo, CmpOp::Gte)?;
            let hi_mask = compare_lit(col, hi, CmpOp::Lte)?;
            Ok(and(&lo_mask, &hi_mask)?)
        }
        Predicate::In { column, values } => {
            let col = get_column(batch, column)?;
            let mut mask = BooleanArray::from(vec![false; n]);
            for val in values {
                let eq_mask = compare_lit(col, val, CmpOp::Eq)?;
                mask = or(&mask, &eq_mask)?;
            }
            Ok(mask)
        }
        Predicate::NotIn { column, values } => {
            let in_pred = Predicate::In {
                column: column.clone(),
                values: values.clone(),
            };
            let in_mask = predicate_to_bool_array(batch, &in_pred)?;
            Ok(not(&in_mask)?)
        }
        Predicate::Contains { column, value } => {
            string_match(batch, column, |s| s.contains(value.as_str()))
        }
        Predicate::StartsWith { column, value } => {
            string_match(batch, column, |s| s.starts_with(value.as_str()))
        }
        Predicate::EndsWith { column, value } => {
            string_match(batch, column, |s| s.ends_with(value.as_str()))
        }
        Predicate::Regex { column, pattern } => {
            let re = regex::Regex::new(pattern)
                .map_err(|e| EngineError::Query(format!("invalid regex: {e}")))?;
            string_match(batch, column, |s| re.is_match(s))
        }
        Predicate::And { exprs } => {
            if exprs.is_empty() {
                return Ok(BooleanArray::from(vec![true; n]));
            }
            let mut result = predicate_to_bool_array(batch, &exprs[0])?;
            for expr in &exprs[1..] {
                let m = predicate_to_bool_array(batch, expr)?;
                result = and(&result, &m)?;
            }
            Ok(result)
        }
        Predicate::Or { exprs } => {
            if exprs.is_empty() {
                return Ok(BooleanArray::from(vec![false; n]));
            }
            let mut result = predicate_to_bool_array(batch, &exprs[0])?;
            for expr in &exprs[1..] {
                let m = predicate_to_bool_array(batch, expr)?;
                result = or(&result, &m)?;
            }
            Ok(result)
        }
        Predicate::Not { expr } => {
            let m = predicate_to_bool_array(batch, expr)?;
            Ok(not(&m)?)
        }
    }
}

pub fn filter_expr_to_bool_array(
    batch: &RecordBatch,
    filter: &FilterExpr,
) -> Result<BooleanArray, EngineError> {
    let n = batch.num_rows();
    match filter {
        FilterExpr::Eq { column, value } => {
            let col = get_column(batch, column)?;
            compare_json(col, value, CmpOp::Eq)
        }
        FilterExpr::Ne { column, value } => {
            let col = get_column(batch, column)?;
            compare_json(col, value, CmpOp::Ne)
        }
        FilterExpr::Gt { column, value } => {
            let col = get_column(batch, column)?;
            compare_json(col, value, CmpOp::Gt)
        }
        FilterExpr::Gte { column, value } => {
            let col = get_column(batch, column)?;
            compare_json(col, value, CmpOp::Gte)
        }
        FilterExpr::Lt { column, value } => {
            let col = get_column(batch, column)?;
            compare_json(col, value, CmpOp::Lt)
        }
        FilterExpr::Lte { column, value } => {
            let col = get_column(batch, column)?;
            compare_json(col, value, CmpOp::Lte)
        }
        FilterExpr::Contains { column, value } => {
            string_match(batch, column, |s| s.contains(value.as_str()))
        }
        FilterExpr::IsNull { column } => {
            let col = get_column(batch, column)?;
            Ok(is_null(col.as_ref())?)
        }
        FilterExpr::IsNotNull { column } => {
            let col = get_column(batch, column)?;
            Ok(is_not_null(col.as_ref())?)
        }
        FilterExpr::And { exprs } => {
            if exprs.is_empty() {
                return Ok(BooleanArray::from(vec![true; n]));
            }
            let mut result = filter_expr_to_bool_array(batch, &exprs[0])?;
            for expr in &exprs[1..] {
                let m = filter_expr_to_bool_array(batch, expr)?;
                result = and(&result, &m)?;
            }
            Ok(result)
        }
        FilterExpr::Or { exprs } => {
            if exprs.is_empty() {
                return Ok(BooleanArray::from(vec![false; n]));
            }
            let mut result = filter_expr_to_bool_array(batch, &exprs[0])?;
            for expr in &exprs[1..] {
                let m = filter_expr_to_bool_array(batch, expr)?;
                result = or(&result, &m)?;
            }
            Ok(result)
        }
        FilterExpr::Not { expr } => {
            let m = filter_expr_to_bool_array(batch, expr)?;
            Ok(not(&m)?)
        }
    }
}

fn apply_filter(
    batches: Vec<RecordBatch>,
    predicate: &Predicate,
) -> Result<Vec<RecordBatch>, EngineError> {
    batches
        .into_iter()
        .map(|batch| {
            let mask = predicate_to_bool_array(&batch, predicate)?;
            Ok(filter_record_batch(&batch, &mask)?)
        })
        .collect()
}

pub fn apply_filter_expr(
    batches: Vec<RecordBatch>,
    filter: &FilterExpr,
) -> Result<Vec<RecordBatch>, EngineError> {
    batches
        .into_iter()
        .map(|batch| {
            let mask = filter_expr_to_bool_array(&batch, filter)?;
            Ok(filter_record_batch(&batch, &mask)?)
        })
        .collect()
}

fn apply_select(
    batches: Vec<RecordBatch>,
    columns: &[String],
) -> Result<Vec<RecordBatch>, EngineError> {
    batches
        .into_iter()
        .map(|batch| {
            let schema = batch.schema();
            let indices: Vec<usize> = columns
                .iter()
                .filter_map(|name| schema.index_of(name).ok())
                .collect();
            Ok(batch.project(&indices)?)
        })
        .collect()
}

fn apply_drop(
    batches: Vec<RecordBatch>,
    columns: &[String],
) -> Result<Vec<RecordBatch>, EngineError> {
    let excluded: HashSet<&String> = columns.iter().collect();
    batches
        .into_iter()
        .map(|batch| {
            let schema = batch.schema();
            let indices: Vec<usize> = (0..schema.fields().len())
                .filter(|&i| !excluded.contains(schema.field(i).name()))
                .collect();
            Ok(batch.project(&indices)?)
        })
        .collect()
}

pub fn apply_sort(
    batches: Vec<RecordBatch>,
    keys: &[SortKey],
) -> Result<Vec<RecordBatch>, EngineError> {
    if batches.is_empty() {
        return Ok(batches);
    }
    let schema = batches[0].schema();
    let combined = concat_batches(&schema, &batches)?;
    let n = combined.num_rows();
    if n == 0 {
        return Ok(vec![combined]);
    }

    let sort_columns: Vec<SortColumn> = keys
        .iter()
        .filter_map(|key| {
            let idx = combined.schema().index_of(&key.column).ok()?;
            Some(SortColumn {
                values: combined.column(idx).clone(),
                options: Some(SortOptions {
                    descending: key.descending,
                    nulls_first: !key.nulls_last,
                }),
            })
        })
        .collect();

    if sort_columns.is_empty() {
        return Ok(vec![combined]);
    }

    let indices = lexsort_to_indices(&sort_columns, None)?;
    let sorted_cols: Vec<ArrayRef> = combined
        .columns()
        .iter()
        .map(|col| take(col.as_ref(), &indices, None))
        .collect::<Result<_, _>>()?;

    Ok(vec![RecordBatch::try_new(schema, sorted_cols)?])
}

pub fn apply_sort_spec(
    batches: Vec<RecordBatch>,
    sort: &tv_core::SortSpec,
) -> Result<Vec<RecordBatch>, EngineError> {
    let key = SortKey {
        column: sort.column.clone(),
        descending: sort.descending,
        nulls_last: true,
    };
    apply_sort(batches, &[key])
}

fn eval_scalar(batch: &RecordBatch, expr: &ScalarExpr) -> Result<ArrayRef, EngineError> {
    let n = batch.num_rows();
    match expr {
        ScalarExpr::Column { name } => {
            let col = get_column(batch, name)?;
            Ok(col.clone())
        }
        ScalarExpr::Literal { value } => {
            let arr: ArrayRef = match value {
                Literal::Null => Arc::new(arrow::array::new_null_array(&ArrowDataType::Null, n)),
                Literal::Bool(b) => Arc::new(BooleanArray::from(vec![*b; n])),
                Literal::Int(i) => Arc::new(Int64Array::from(vec![*i; n])),
                Literal::Float(f) => Arc::new(Float64Array::from(vec![*f; n])),
                Literal::Text(s) => Arc::new(StringArray::from(vec![s.as_str(); n])),
            };
            Ok(arr)
        }
        ScalarExpr::BinOp { op, left, right } => {
            let l = eval_scalar(batch, left)?;
            let r = eval_scalar(batch, right)?;
            let l_f64 = cast(l.as_ref(), &ArrowDataType::Float64)?;
            let r_f64 = cast(r.as_ref(), &ArrowDataType::Float64)?;
            let l_arr = l_f64.as_any().downcast_ref::<Float64Array>().unwrap();
            let r_arr = r_f64.as_any().downcast_ref::<Float64Array>().unwrap();
            let result: Vec<Option<f64>> = (0..n)
                .map(|i| {
                    if l_arr.is_null(i) || r_arr.is_null(i) {
                        return None;
                    }
                    let lv = l_arr.value(i);
                    let rv = r_arr.value(i);
                    Some(match op {
                        BinOp::Add => lv + rv,
                        BinOp::Sub => lv - rv,
                        BinOp::Mul => lv * rv,
                        BinOp::Div => {
                            if rv == 0.0 {
                                f64::NAN
                            } else {
                                lv / rv
                            }
                        }
                        BinOp::Mod => {
                            if rv == 0.0 {
                                f64::NAN
                            } else {
                                lv % rv
                            }
                        }
                    })
                })
                .collect();
            Ok(Arc::new(Float64Array::from(result)))
        }
        ScalarExpr::Abs(inner) => {
            let arr = eval_scalar(batch, inner)?;
            let f64_arr = cast(arr.as_ref(), &ArrowDataType::Float64)?;
            let f64_arr = f64_arr.as_any().downcast_ref::<Float64Array>().unwrap();
            let result: Vec<Option<f64>> = (0..f64_arr.len())
                .map(|i| {
                    if f64_arr.is_null(i) {
                        None
                    } else {
                        Some(f64_arr.value(i).abs())
                    }
                })
                .collect();
            Ok(Arc::new(Float64Array::from(result)))
        }
        ScalarExpr::Round { expr, decimals } => {
            let arr = eval_scalar(batch, expr)?;
            let f64_arr = cast(arr.as_ref(), &ArrowDataType::Float64)?;
            let f64_arr = f64_arr.as_any().downcast_ref::<Float64Array>().unwrap();
            let factor = 10f64.powi(*decimals);
            let result: Vec<Option<f64>> = (0..f64_arr.len())
                .map(|i| {
                    if f64_arr.is_null(i) {
                        None
                    } else {
                        Some((f64_arr.value(i) * factor).round() / factor)
                    }
                })
                .collect();
            Ok(Arc::new(Float64Array::from(result)))
        }
        ScalarExpr::Floor(inner) => {
            let arr = eval_scalar(batch, inner)?;
            let f64_arr = cast(arr.as_ref(), &ArrowDataType::Float64)?;
            let f64_arr = f64_arr.as_any().downcast_ref::<Float64Array>().unwrap();
            let result: Vec<Option<f64>> = (0..f64_arr.len())
                .map(|i| {
                    if f64_arr.is_null(i) {
                        None
                    } else {
                        Some(f64_arr.value(i).floor())
                    }
                })
                .collect();
            Ok(Arc::new(Float64Array::from(result)))
        }
        ScalarExpr::Ceil(inner) => {
            let arr = eval_scalar(batch, inner)?;
            let f64_arr = cast(arr.as_ref(), &ArrowDataType::Float64)?;
            let f64_arr = f64_arr.as_any().downcast_ref::<Float64Array>().unwrap();
            let result: Vec<Option<f64>> = (0..f64_arr.len())
                .map(|i| {
                    if f64_arr.is_null(i) {
                        None
                    } else {
                        Some(f64_arr.value(i).ceil())
                    }
                })
                .collect();
            Ok(Arc::new(Float64Array::from(result)))
        }
        ScalarExpr::Upper(inner) => {
            let arr = eval_scalar(batch, inner)?;
            let str_col = cast(arr.as_ref(), &ArrowDataType::Utf8)?;
            let str_arr = str_col.as_any().downcast_ref::<StringArray>().unwrap();
            let result: Vec<Option<String>> = (0..str_arr.len())
                .map(|i| {
                    if str_arr.is_null(i) {
                        None
                    } else {
                        Some(str_arr.value(i).to_uppercase())
                    }
                })
                .collect();
            Ok(Arc::new(StringArray::from(result)))
        }
        ScalarExpr::Lower(inner) => {
            let arr = eval_scalar(batch, inner)?;
            let str_col = cast(arr.as_ref(), &ArrowDataType::Utf8)?;
            let str_arr = str_col.as_any().downcast_ref::<StringArray>().unwrap();
            let result: Vec<Option<String>> = (0..str_arr.len())
                .map(|i| {
                    if str_arr.is_null(i) {
                        None
                    } else {
                        Some(str_arr.value(i).to_lowercase())
                    }
                })
                .collect();
            Ok(Arc::new(StringArray::from(result)))
        }
        ScalarExpr::Trim(inner) => {
            let arr = eval_scalar(batch, inner)?;
            let str_col = cast(arr.as_ref(), &ArrowDataType::Utf8)?;
            let str_arr = str_col.as_any().downcast_ref::<StringArray>().unwrap();
            let result: Vec<Option<String>> = (0..str_arr.len())
                .map(|i| {
                    if str_arr.is_null(i) {
                        None
                    } else {
                        Some(str_arr.value(i).trim().to_string())
                    }
                })
                .collect();
            Ok(Arc::new(StringArray::from(result)))
        }
        ScalarExpr::Length(inner) => {
            let arr = eval_scalar(batch, inner)?;
            let str_col = cast(arr.as_ref(), &ArrowDataType::Utf8)?;
            let str_arr = str_col.as_any().downcast_ref::<StringArray>().unwrap();
            let result: Vec<Option<i32>> = (0..str_arr.len())
                .map(|i| {
                    if str_arr.is_null(i) {
                        None
                    } else {
                        Some(str_arr.value(i).len() as i32)
                    }
                })
                .collect();
            Ok(Arc::new(Int32Array::from(result)))
        }
        ScalarExpr::Substr { expr, start, len } => {
            let arr = eval_scalar(batch, expr)?;
            let str_col = cast(arr.as_ref(), &ArrowDataType::Utf8)?;
            let str_arr = str_col.as_any().downcast_ref::<StringArray>().unwrap();
            let begin = *start as usize;
            let result: Vec<Option<String>> = (0..str_arr.len())
                .map(|i| {
                    if str_arr.is_null(i) {
                        return None;
                    }
                    let chars: Vec<char> = str_arr.value(i).chars().collect();
                    let s = begin.min(chars.len());
                    let e = len
                        .map(|l| (s + l as usize).min(chars.len()))
                        .unwrap_or(chars.len());
                    Some(chars[s..e].iter().collect())
                })
                .collect();
            Ok(Arc::new(StringArray::from(result)))
        }
        ScalarExpr::Concat { parts } => {
            let arrays: Vec<ArrayRef> = parts
                .iter()
                .map(|p| {
                    let arr = eval_scalar(batch, p)?;
                    cast(arr.as_ref(), &ArrowDataType::Utf8).map_err(EngineError::Arrow)
                })
                .collect::<Result<_, _>>()?;

            let result: Vec<Option<String>> = (0..n)
                .map(|i| {
                    let mut s = String::new();
                    for arr in &arrays {
                        let str_arr = arr.as_any().downcast_ref::<StringArray>().unwrap();
                        if str_arr.is_null(i) {
                            return None;
                        }
                        s.push_str(str_arr.value(i));
                    }
                    Some(s)
                })
                .collect();
            Ok(Arc::new(StringArray::from(result)))
        }
        ScalarExpr::Cast { expr, to_type } => {
            let arr = eval_scalar(batch, expr)?;
            let target = match to_type {
                DataType::Int32 => ArrowDataType::Int32,
                DataType::Int64 => ArrowDataType::Int64,
                DataType::Float32 => ArrowDataType::Float32,
                DataType::Float64 => ArrowDataType::Float64,
                DataType::Text => ArrowDataType::Utf8,
                DataType::Boolean => ArrowDataType::Boolean,
                DataType::Date => ArrowDataType::Date32,
                DataType::Timestamp => {
                    ArrowDataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None)
                }
            };
            Ok(cast(arr.as_ref(), &target)?)
        }
        ScalarExpr::Year(inner) => {
            let arr = eval_scalar(batch, inner)?;
            Ok(Arc::new(arrow::compute::date_part(
                arr.as_ref(),
                arrow::compute::DatePart::Year,
            )?))
        }
        ScalarExpr::Month(inner) => {
            let arr = eval_scalar(batch, inner)?;
            Ok(Arc::new(arrow::compute::date_part(
                arr.as_ref(),
                arrow::compute::DatePart::Month,
            )?))
        }
        ScalarExpr::Day(inner) => {
            let arr = eval_scalar(batch, inner)?;
            Ok(Arc::new(arrow::compute::date_part(
                arr.as_ref(),
                arrow::compute::DatePart::Day,
            )?))
        }
        ScalarExpr::Coalesce { exprs } => {
            if exprs.is_empty() {
                return Ok(Arc::new(arrow::array::new_null_array(
                    &ArrowDataType::Null,
                    n,
                )));
            }
            let arrays: Vec<ArrayRef> = exprs
                .iter()
                .map(|e| eval_scalar(batch, e))
                .collect::<Result<_, _>>()?;
            let result: Vec<Option<String>> = (0..n)
                .map(|i| {
                    for arr in &arrays {
                        if !arr.is_null(i) {
                            let str_col = cast(arr.as_ref(), &ArrowDataType::Utf8).ok()?;
                            let s = str_col.as_any().downcast_ref::<StringArray>()?;
                            return Some(s.value(i).to_string());
                        }
                    }
                    None
                })
                .collect();
            Ok(Arc::new(StringArray::from(result)))
        }
        ScalarExpr::Case { whens, else_expr } => {
            let mut result: Vec<Option<String>> = vec![None; n];
            let mut assigned = vec![false; n];

            for (pred, then_expr) in whens {
                let mask = predicate_to_bool_array(batch, pred)?;
                let then_arr = eval_scalar(batch, then_expr)?;
                let then_str = cast(then_arr.as_ref(), &ArrowDataType::Utf8)?;
                let then_sarr = then_str.as_any().downcast_ref::<StringArray>().unwrap();
                for i in 0..n {
                    if !assigned[i] && mask.is_valid(i) && mask.value(i) {
                        result[i] = if then_sarr.is_null(i) {
                            None
                        } else {
                            Some(then_sarr.value(i).to_string())
                        };
                        assigned[i] = true;
                    }
                }
            }

            if let Some(else_e) = else_expr {
                let else_arr = eval_scalar(batch, else_e)?;
                let else_str = cast(else_arr.as_ref(), &ArrowDataType::Utf8)?;
                let else_sarr = else_str.as_any().downcast_ref::<StringArray>().unwrap();
                for i in 0..n {
                    if !assigned[i] {
                        result[i] = if else_sarr.is_null(i) {
                            None
                        } else {
                            Some(else_sarr.value(i).to_string())
                        };
                    }
                }
            }

            Ok(Arc::new(StringArray::from(result)))
        }
        ScalarExpr::Rank { order } => {
            let sort_keys: Vec<SortKey> = order.to_vec();
            let sort_cols: Vec<SortColumn> = sort_keys
                .iter()
                .filter_map(|key| {
                    let idx = batch.schema().index_of(&key.column).ok()?;
                    Some(SortColumn {
                        values: batch.column(idx).clone(),
                        options: Some(SortOptions {
                            descending: key.descending,
                            nulls_first: !key.nulls_last,
                        }),
                    })
                })
                .collect();
            if sort_cols.is_empty() {
                let ranks: Vec<i64> = (1..=(n as i64)).collect();
                return Ok(Arc::new(Int64Array::from(ranks)));
            }
            let indices = lexsort_to_indices(&sort_cols, None)?;
            let mut rank_of = vec![0i64; n];
            for (rank, &orig_idx) in indices.values().iter().enumerate() {
                rank_of[orig_idx as usize] = (rank + 1) as i64;
            }
            Ok(Arc::new(Int64Array::from(rank_of)))
        }
        ScalarExpr::NTile { n: n_tiles } => {
            let n_tiles_val = *n_tiles as usize;
            if n_tiles_val == 0 || n == 0 {
                return Ok(Arc::new(Int64Array::from(vec![1i64; n])));
            }
            let buckets: Vec<i64> = (0..n)
                .map(|i| {
                    let bucket = (i * n_tiles_val / n) + 1;
                    bucket.min(n_tiles_val) as i64
                })
                .collect();
            Ok(Arc::new(Int64Array::from(buckets)))
        }
    }
}

fn apply_derive(
    batches: Vec<RecordBatch>,
    name: &str,
    expr: &ScalarExpr,
) -> Result<Vec<RecordBatch>, EngineError> {
    batches
        .into_iter()
        .map(|batch| {
            let new_col = eval_scalar(&batch, expr)?;
            let mut fields: Vec<Arc<Field>> = batch.schema().fields().iter().cloned().collect();
            let new_field = Arc::new(Field::new(name, new_col.data_type().clone(), true));
            fields.push(new_field);
            let new_schema = Arc::new(Schema::new(fields));
            let mut cols: Vec<ArrayRef> = batch.columns().to_vec();
            cols.push(new_col);
            Ok(RecordBatch::try_new(new_schema, cols)?)
        })
        .collect()
}

fn format_array_value(arr: &ArrayRef, idx: usize) -> String {
    if arr.is_null(idx) {
        return String::from("\x00NULL");
    }
    match cast(arr.as_ref(), &ArrowDataType::Utf8) {
        Ok(str_col) => {
            if let Some(s) = str_col.as_any().downcast_ref::<StringArray>() {
                if !s.is_null(idx) {
                    return s.value(idx).to_string();
                }
            }
            String::new()
        }
        Err(_) => format!("__idx_{idx}"),
    }
}

fn apply_deduplicate(
    batches: Vec<RecordBatch>,
    columns: &Option<Vec<String>>,
) -> Result<Vec<RecordBatch>, EngineError> {
    if batches.is_empty() {
        return Ok(batches);
    }
    let schema = batches[0].schema();
    let combined = concat_batches(&schema, &batches)?;
    let n = combined.num_rows();
    if n == 0 {
        return Ok(vec![combined]);
    }

    let key_cols: Vec<(usize, ArrayRef)> = match columns {
        None => (0..combined.schema().fields().len())
            .map(|i| (i, combined.column(i).clone()))
            .collect(),
        Some(col_names) => col_names
            .iter()
            .filter_map(|name| {
                combined
                    .schema()
                    .index_of(name)
                    .ok()
                    .map(|i| (i, combined.column(i).clone()))
            })
            .collect(),
    };

    let mut seen: HashSet<String> = HashSet::new();
    let mut keep: Vec<bool> = Vec::with_capacity(n);

    for row in 0..n {
        let key = key_cols
            .iter()
            .map(|(_, col)| format_array_value(col, row))
            .collect::<Vec<_>>()
            .join("\x01");
        keep.push(seen.insert(key));
    }

    let mask = BooleanArray::from(keep);
    Ok(vec![filter_record_batch(&combined, &mask)?])
}

fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

fn apply_sample(
    batches: Vec<RecordBatch>,
    n: u64,
    seed: Option<u64>,
) -> Result<Vec<RecordBatch>, EngineError> {
    if batches.is_empty() {
        return Ok(batches);
    }
    let schema = batches[0].schema();
    let combined = concat_batches(&schema, &batches)?;
    let total = combined.num_rows();
    let take_n = (n as usize).min(total);
    if take_n == total {
        return Ok(vec![combined]);
    }
    let mut indices: Vec<u32> = (0..total as u32).collect();
    let mut rng = seed.unwrap_or(0x517cc1b727220a95_u64);
    for i in 0..take_n {
        rng = mix64(rng);
        let j = i + (rng as usize) % (total - i);
        indices.swap(i, j);
    }
    indices.truncate(take_n);
    let idx_arr = UInt32Array::from(indices);
    let cols: Vec<ArrayRef> = combined
        .columns()
        .iter()
        .map(|col| take(col.as_ref(), &idx_arr, None).map_err(EngineError::Arrow))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(vec![RecordBatch::try_new(schema, cols)?])
}

fn apply_rename(
    batches: Vec<RecordBatch>,
    mappings: &[(String, String)],
) -> Result<Vec<RecordBatch>, EngineError> {
    batches
        .into_iter()
        .map(|batch| {
            let new_fields: Vec<Arc<Field>> = batch
                .schema()
                .fields()
                .iter()
                .map(|f| {
                    if let Some((_, to)) = mappings.iter().find(|(from, _)| from == f.name()) {
                        Arc::new(Field::new(to, f.data_type().clone(), f.is_nullable()))
                    } else {
                        f.clone()
                    }
                })
                .collect();
            let new_schema = Arc::new(Schema::new(new_fields));
            Ok(RecordBatch::try_new(new_schema, batch.columns().to_vec())?)
        })
        .collect()
}

fn apply_limit(batches: Vec<RecordBatch>, n: u64) -> Result<Vec<RecordBatch>, EngineError> {
    if batches.is_empty() {
        return Ok(batches);
    }
    let schema = batches[0].schema();
    let combined = concat_batches(&schema, &batches)?;
    let take_n = (n as usize).min(combined.num_rows());
    Ok(vec![combined.slice(0, take_n)])
}

fn apply_group_by(
    batches: Vec<RecordBatch>,
    keys: &[String],
    aggs: &[AggExpr],
) -> Result<Vec<RecordBatch>, EngineError> {
    if batches.is_empty() {
        return Ok(batches);
    }
    let schema = batches[0].schema();
    let combined = concat_batches(&schema, &batches)?;

    let sorted = if !keys.is_empty() {
        let sort_keys: Vec<SortKey> = keys
            .iter()
            .map(|k| SortKey {
                column: k.clone(),
                descending: false,
                nulls_last: true,
            })
            .collect();
        let sorted_batches = apply_sort(vec![combined], &sort_keys)?;
        concat_batches(&schema, &sorted_batches)?
    } else {
        combined
    };

    group_by_sorted(&sorted, keys, aggs)
}

fn group_by_sorted(
    batch: &RecordBatch,
    keys: &[String],
    aggs: &[AggExpr],
) -> Result<Vec<RecordBatch>, EngineError> {
    let n = batch.num_rows();

    let key_indices: Vec<usize> = keys
        .iter()
        .filter_map(|k| batch.schema().index_of(k).ok())
        .collect();

    let boundaries: Vec<(usize, usize)> = if n == 0 {
        vec![]
    } else if key_indices.is_empty() {
        vec![(0, n)]
    } else {
        let mut bds = Vec::new();
        let mut start = 0usize;
        for i in 1..n {
            let changed = key_indices.iter().any(|&idx| {
                let col = batch.column(idx);
                format_array_value(col, i) != format_array_value(col, i - 1)
            });
            if changed {
                bds.push((start, i));
                start = i;
            }
        }
        bds.push((start, n));
        bds
    };

    let n_groups = boundaries.len();

    let mut out_cols: Vec<ArrayRef> = Vec::new();
    let mut out_fields: Vec<Arc<Field>> = Vec::new();

    for &ki in &key_indices {
        let col = batch.column(ki);
        let group_rows: Vec<u32> = boundaries.iter().map(|(s, _)| *s as u32).collect();
        let indices = UInt32Array::from(group_rows);
        out_cols.push(take(col.as_ref(), &indices, None)?);
        out_fields.push(Arc::new(batch.schema().field(ki).clone()));
    }

    for agg in aggs {
        let (col, field) = compute_agg(batch, agg, &boundaries)?;
        out_cols.push(col);
        out_fields.push(Arc::new(field));
    }

    let out_schema = Arc::new(Schema::new(out_fields));
    if n_groups == 0 {
        let empty_cols: Vec<ArrayRef> = out_schema
            .fields()
            .iter()
            .map(|f| Arc::new(arrow::array::new_null_array(f.data_type(), 0)) as ArrayRef)
            .collect();
        return Ok(vec![RecordBatch::try_new(out_schema, empty_cols)?]);
    }
    Ok(vec![RecordBatch::try_new(out_schema, out_cols)?])
}

fn compute_agg(
    batch: &RecordBatch,
    agg: &AggExpr,
    boundaries: &[(usize, usize)],
) -> Result<(ArrayRef, Field), EngineError> {
    let alias = tv_core::agg_alias(agg).to_string();

    match agg {
        AggExpr::Count { .. } => {
            let counts: Vec<i64> = boundaries.iter().map(|(s, e)| (e - s) as i64).collect();
            Ok((
                Arc::new(Int64Array::from(counts)),
                Field::new(&alias, ArrowDataType::Int64, false),
            ))
        }
        AggExpr::CountDistinct { column, .. } => {
            let col_idx = batch
                .schema()
                .index_of(column)
                .map_err(|_| EngineError::Query(format!("column {column} not found")))?;
            let col = batch.column(col_idx);
            let counts: Vec<i64> = boundaries
                .iter()
                .map(|(s, e)| {
                    let distinct: HashSet<String> =
                        (*s..*e).map(|i| format_array_value(col, i)).collect();
                    distinct.len() as i64
                })
                .collect();
            Ok((
                Arc::new(Int64Array::from(counts)),
                Field::new(&alias, ArrowDataType::Int64, true),
            ))
        }
        AggExpr::Sum { column, .. }
        | AggExpr::Min { column, .. }
        | AggExpr::Max { column, .. }
        | AggExpr::Mean { column, .. }
        | AggExpr::Median { column, .. }
        | AggExpr::StdDev { column, .. }
        | AggExpr::Percentile { column, .. } => {
            let col_idx = batch
                .schema()
                .index_of(column)
                .map_err(|_| EngineError::Query(format!("column {column} not found")))?;
            let col = batch.column(col_idx);
            let float_col = cast(col.as_ref(), &ArrowDataType::Float64)?;
            let float_arr = float_col.as_any().downcast_ref::<Float64Array>().unwrap();

            let values: Vec<Option<f64>> = boundaries
                .iter()
                .map(|(s, e)| {
                    let vals: Vec<f64> = (*s..*e)
                        .filter(|&i| !float_arr.is_null(i))
                        .map(|i| float_arr.value(i))
                        .collect();

                    if vals.is_empty() {
                        return None;
                    }

                    Some(match agg {
                        AggExpr::Sum { .. } => vals.iter().sum(),
                        AggExpr::Min { .. } => vals.iter().cloned().fold(f64::MAX, f64::min),
                        AggExpr::Max { .. } => vals.iter().cloned().fold(f64::MIN, f64::max),
                        AggExpr::Mean { .. } => vals.iter().sum::<f64>() / vals.len() as f64,
                        AggExpr::Median { .. } => {
                            let mut sorted = vals.clone();
                            sorted.sort_by(|a, b| a.total_cmp(b));
                            let mid = sorted.len() / 2;
                            if sorted.len() % 2 == 0 {
                                (sorted[mid - 1] + sorted[mid]) / 2.0
                            } else {
                                sorted[mid]
                            }
                        }
                        AggExpr::StdDev { .. } => {
                            if vals.len() < 2 {
                                return None;
                            }
                            let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                            let variance = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                                / (vals.len() - 1) as f64;
                            variance.sqrt()
                        }
                        AggExpr::Percentile { p, .. } => {
                            let mut sorted = vals.clone();
                            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                            let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
                            sorted[idx.min(sorted.len() - 1)]
                        }
                        _ => unreachable!(),
                    })
                })
                .collect();

            Ok((
                Arc::new(Float64Array::from(values)),
                Field::new(&alias, ArrowDataType::Float64, true),
            ))
        }
    }
}

pub fn required_columns(
    _ops: &[ViewOp],
    col_offset: usize,
    cols: usize,
    n_total: usize,
) -> Vec<usize> {
    let col_end = (col_offset + cols).min(n_total);
    (col_offset..col_end).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::datatypes::{DataType as ArrowDataType, Field, Schema};
    use std::sync::Arc;

    fn make_int_batch(n: i64) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", ArrowDataType::Int64, false),
            Field::new("score", ArrowDataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from((0..n).collect::<Vec<_>>())),
                Arc::new(Float64Array::from(
                    (0..n).map(|i| i as f64 * 1.5).collect::<Vec<_>>(),
                )),
            ],
        )
        .unwrap()
    }

    fn make_str_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", ArrowDataType::Utf8, false),
            Field::new("val", ArrowDataType::Int64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["alice", "bob", "charlie", "alice"])),
                Arc::new(Int64Array::from(vec![10i64, 20, 30, 40])),
            ],
        )
        .unwrap()
    }

    fn make_nullable_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", ArrowDataType::Float64, true),
            Field::new("y", ArrowDataType::Utf8, true),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![Some(1.0), None, Some(3.0), None])),
                Arc::new(StringArray::from(vec![Some("a"), Some("b"), None, None])),
            ],
        )
        .unwrap()
    }

    fn total_rows(batches: &[RecordBatch]) -> usize {
        batches.iter().map(|b| b.num_rows()).sum()
    }

    #[allow(dead_code)]
    fn col_i64(batch: &RecordBatch, name: &str) -> Vec<i64> {
        let idx = batch.schema().index_of(name).unwrap();
        let arr = batch
            .column(idx)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        (0..arr.len()).map(|i| arr.value(i)).collect()
    }

    fn col_f64(batch: &RecordBatch, name: &str) -> Vec<f64> {
        let idx = batch.schema().index_of(name).unwrap();
        let arr = batch
            .column(idx)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        (0..arr.len()).map(|i| arr.value(i)).collect()
    }

    fn col_str(batch: &RecordBatch, name: &str) -> Vec<String> {
        let idx = batch.schema().index_of(name).unwrap();
        let arr = batch
            .column(idx)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        (0..arr.len()).map(|i| arr.value(i).to_string()).collect()
    }

    #[test]
    fn predicate_eq_int() {
        let batch = make_int_batch(5);
        let pred = Predicate::Eq {
            column: "id".into(),
            value: Literal::Int(2),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        let kept: Vec<bool> = (0..mask.len()).map(|i| mask.value(i)).collect();
        assert_eq!(kept, vec![false, false, true, false, false]);
    }

    #[test]
    fn predicate_eq_string() {
        let batch = make_str_batch();
        let pred = Predicate::Eq {
            column: "name".into(),
            value: Literal::Text("alice".into()),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        let kept: Vec<bool> = (0..mask.len()).map(|i| mask.value(i)).collect();
        assert_eq!(kept, vec![true, false, false, true]);
    }

    #[test]
    fn predicate_ne_float() {
        let batch = make_int_batch(4);
        let pred = Predicate::Ne {
            column: "score".into(),
            value: Literal::Float(0.0),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        assert!(!mask.value(0));
        assert!(mask.value(1));
    }

    #[test]
    fn predicate_gt_int() {
        let batch = make_int_batch(5);
        let pred = Predicate::Gt {
            column: "id".into(),
            value: Literal::Int(2),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        let count = (0..mask.len()).filter(|&i| mask.value(i)).count();
        assert_eq!(count, 2);
    }

    #[test]
    fn predicate_lt_int() {
        let batch = make_int_batch(5);
        let pred = Predicate::Lt {
            column: "id".into(),
            value: Literal::Int(3),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        let count = (0..mask.len()).filter(|&i| mask.value(i)).count();
        assert_eq!(count, 3);
    }

    #[test]
    fn predicate_gte_int() {
        let batch = make_int_batch(5);
        let pred = Predicate::Gte {
            column: "id".into(),
            value: Literal::Int(3),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        let count = (0..mask.len()).filter(|&i| mask.value(i)).count();
        assert_eq!(count, 2);
    }

    #[test]
    fn predicate_lte_int() {
        let batch = make_int_batch(5);
        let pred = Predicate::Lte {
            column: "id".into(),
            value: Literal::Int(2),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        let count = (0..mask.len()).filter(|&i| mask.value(i)).count();
        assert_eq!(count, 3);
    }

    #[test]
    fn predicate_between_float() {
        let batch = make_int_batch(10);
        let pred = Predicate::Between {
            column: "score".into(),
            lo: Literal::Float(3.0),
            hi: Literal::Float(7.5),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        let ids: Vec<usize> = (0..mask.len()).filter(|&i| mask.value(i)).collect();
        assert!(!ids.is_empty());
        for i in ids {
            let v = i as f64 * 1.5;
            assert!(v >= 3.0 && v <= 7.5);
        }
    }

    #[test]
    fn predicate_in_list() {
        let batch = make_str_batch();
        let pred = Predicate::In {
            column: "name".into(),
            values: vec![
                Literal::Text("alice".into()),
                Literal::Text("charlie".into()),
            ],
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        assert!(mask.value(0));
        assert!(!mask.value(1));
        assert!(mask.value(2));
        assert!(mask.value(3));
    }

    #[test]
    fn predicate_not_in() {
        let batch = make_str_batch();
        let pred = Predicate::NotIn {
            column: "name".into(),
            values: vec![Literal::Text("alice".into())],
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        assert!(!mask.value(0));
        assert!(mask.value(1));
        assert!(mask.value(2));
        assert!(!mask.value(3));
    }

    #[test]
    fn predicate_contains() {
        let batch = make_str_batch();
        let pred = Predicate::Contains {
            column: "name".into(),
            value: "li".into(),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        assert!(mask.value(0));
        assert!(!mask.value(1));
        assert!(mask.value(2));
    }

    #[test]
    fn predicate_starts_with() {
        let batch = make_str_batch();
        let pred = Predicate::StartsWith {
            column: "name".into(),
            value: "ali".into(),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        assert!(mask.value(0));
        assert!(!mask.value(1));
        assert!(!mask.value(2));
    }

    #[test]
    fn predicate_ends_with() {
        let batch = make_str_batch();
        let pred = Predicate::EndsWith {
            column: "name".into(),
            value: "ice".into(),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        assert!(mask.value(0));
        assert!(!mask.value(1));
    }

    #[test]
    fn predicate_regex() {
        let batch = make_str_batch();
        let pred = Predicate::Regex {
            column: "name".into(),
            pattern: "^a.*".into(),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        assert!(mask.value(0));
        assert!(!mask.value(1));
    }

    #[test]
    fn predicate_is_null() {
        let batch = make_nullable_batch();
        let pred = Predicate::IsNull { column: "x".into() };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        assert!(!mask.value(0));
        assert!(mask.value(1));
        assert!(!mask.value(2));
        assert!(mask.value(3));
    }

    #[test]
    fn predicate_is_not_null() {
        let batch = make_nullable_batch();
        let pred = Predicate::IsNotNull { column: "y".into() };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        assert!(mask.value(0));
        assert!(mask.value(1));
        assert!(!mask.value(2));
        assert!(!mask.value(3));
    }

    #[test]
    fn predicate_and_combinator() {
        let batch = make_int_batch(10);
        let pred = Predicate::And {
            exprs: vec![
                Predicate::Gte {
                    column: "id".into(),
                    value: Literal::Int(3),
                },
                Predicate::Lt {
                    column: "id".into(),
                    value: Literal::Int(7),
                },
            ],
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        let count = (0..mask.len()).filter(|&i| mask.value(i)).count();
        assert_eq!(count, 4);
    }

    #[test]
    fn predicate_or_combinator() {
        let batch = make_int_batch(10);
        let pred = Predicate::Or {
            exprs: vec![
                Predicate::Eq {
                    column: "id".into(),
                    value: Literal::Int(0),
                },
                Predicate::Eq {
                    column: "id".into(),
                    value: Literal::Int(9),
                },
            ],
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        let count = (0..mask.len()).filter(|&i| mask.value(i)).count();
        assert_eq!(count, 2);
    }

    #[test]
    fn predicate_not() {
        let batch = make_int_batch(5);
        let pred = Predicate::Not {
            expr: Box::new(Predicate::Lt {
                column: "id".into(),
                value: Literal::Int(3),
            }),
        };
        let mask = predicate_to_bool_array(&batch, &pred).unwrap();
        let count = (0..mask.len()).filter(|&i| mask.value(i)).count();
        assert_eq!(count, 2);
    }

    #[test]
    fn apply_filter_reduces_rows() {
        let batch = make_int_batch(10);
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::Lt {
                column: "id".into(),
                value: Literal::Int(5),
            },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 5);
    }

    #[test]
    fn apply_filter_all_match() {
        let batch = make_int_batch(5);
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::Gte {
                column: "id".into(),
                value: Literal::Int(0),
            },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 5);
    }

    #[test]
    fn apply_filter_none_match() {
        let batch = make_int_batch(5);
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::Gt {
                column: "id".into(),
                value: Literal::Int(100),
            },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 0);
    }

    #[test]
    fn apply_filter_with_nulls() {
        let batch = make_nullable_batch();
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::IsNotNull { column: "x".into() },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 2);
    }

    #[test]
    fn sort_ascending_int() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            ArrowDataType::Int64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Int64Array::from(vec![3i64, 1, 4, 1, 5, 9, 2, 6]))],
        )
        .unwrap();
        let ops = vec![ViewOp::Sort {
            keys: vec![SortKey {
                column: "x".into(),
                descending: false,
                nulls_last: true,
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let arr = result[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        let vals: Vec<i64> = (0..arr.len()).map(|i| arr.value(i)).collect();
        let mut sorted = vals.clone();
        sorted.sort();
        assert_eq!(vals, sorted);
    }

    #[test]
    fn sort_descending_float() {
        let batch = make_int_batch(5);
        let ops = vec![ViewOp::Sort {
            keys: vec![SortKey {
                column: "score".into(),
                descending: true,
                nulls_last: true,
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let vals = col_f64(&result[0], "score");
        for i in 1..vals.len() {
            assert!(vals[i - 1] >= vals[i]);
        }
    }

    #[test]
    fn sort_multi_key() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", ArrowDataType::Int64, false),
            Field::new("b", ArrowDataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![2i64, 1, 2, 1])),
                Arc::new(Int64Array::from(vec![2i64, 2, 1, 1])),
            ],
        )
        .unwrap();
        let ops = vec![ViewOp::Sort {
            keys: vec![
                SortKey {
                    column: "a".into(),
                    descending: false,
                    nulls_last: true,
                },
                SortKey {
                    column: "b".into(),
                    descending: false,
                    nulls_last: true,
                },
            ],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let arr = result[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(arr.value(0), 1);
        assert_eq!(arr.value(2), 2);
    }

    #[test]
    fn sort_with_nulls() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            ArrowDataType::Float64,
            true,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(vec![
                Some(3.0),
                None,
                Some(1.0),
                None,
            ]))],
        )
        .unwrap();
        let ops = vec![ViewOp::Sort {
            keys: vec![SortKey {
                column: "x".into(),
                descending: false,
                nulls_last: true,
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 4);
    }

    #[test]
    fn sort_empty_batches() {
        let result = execute_pipeline(
            vec![],
            &[ViewOp::Sort {
                keys: vec![SortKey {
                    column: "x".into(),
                    descending: false,
                    nulls_last: true,
                }],
            }],
        )
        .unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn group_by_count() {
        let batch = make_str_batch();
        let ops = vec![ViewOp::GroupBy {
            keys: vec!["name".into()],
            aggs: vec![AggExpr::Count {
                alias: "cnt".into(),
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 3);
        let names = col_str(&result[0], "name");
        let alice_idx = names.iter().position(|s| s == "alice").unwrap();
        let arr = result[0].schema().index_of("cnt").unwrap();
        let cnts = result[0]
            .column(arr)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(cnts.value(alice_idx), 2);
    }

    #[test]
    fn group_by_sum() {
        let batch = make_str_batch();
        let ops = vec![ViewOp::GroupBy {
            keys: vec!["name".into()],
            aggs: vec![AggExpr::Sum {
                column: "val".into(),
                alias: "total".into(),
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let names = col_str(&result[0], "name");
        let alice_idx = names.iter().position(|s| s == "alice").unwrap();
        let totals = col_f64(&result[0], "total");
        assert!((totals[alice_idx] - 50.0).abs() < 1e-9);
    }

    #[test]
    fn group_by_min_max() {
        let batch = make_int_batch(10);
        let ops = vec![ViewOp::GroupBy {
            keys: vec![],
            aggs: vec![
                AggExpr::Min {
                    column: "id".into(),
                    alias: "mn".into(),
                },
                AggExpr::Max {
                    column: "id".into(),
                    alias: "mx".into(),
                },
            ],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let mn = col_f64(&result[0], "mn");
        let mx = col_f64(&result[0], "mx");
        assert!((mn[0] - 0.0).abs() < 1e-9);
        assert!((mx[0] - 9.0).abs() < 1e-9);
    }

    #[test]
    fn group_by_mean() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            ArrowDataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(vec![2.0f64, 4.0, 6.0]))],
        )
        .unwrap();
        let ops = vec![ViewOp::GroupBy {
            keys: vec![],
            aggs: vec![AggExpr::Mean {
                column: "x".into(),
                alias: "avg".into(),
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let avg = col_f64(&result[0], "avg");
        assert!((avg[0] - 4.0).abs() < 1e-9);
    }

    #[test]
    fn group_by_median() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            ArrowDataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(vec![1.0f64, 3.0, 2.0]))],
        )
        .unwrap();
        let ops = vec![ViewOp::GroupBy {
            keys: vec![],
            aggs: vec![AggExpr::Median {
                column: "x".into(),
                alias: "med".into(),
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let med = col_f64(&result[0], "med");
        assert!((med[0] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn group_by_std_dev() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            ArrowDataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(vec![
                2.0f64, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0,
            ]))],
        )
        .unwrap();
        let ops = vec![ViewOp::GroupBy {
            keys: vec![],
            aggs: vec![AggExpr::StdDev {
                column: "x".into(),
                alias: "sd".into(),
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let sd = col_f64(&result[0], "sd");
        assert!(sd[0] > 0.0);
    }

    #[test]
    fn group_by_percentile() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            ArrowDataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(
                (1..=100).map(|i| i as f64).collect::<Vec<_>>(),
            ))],
        )
        .unwrap();
        let ops = vec![ViewOp::GroupBy {
            keys: vec![],
            aggs: vec![AggExpr::Percentile {
                column: "x".into(),
                p: 50.0,
                alias: "p50".into(),
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let p50 = col_f64(&result[0], "p50");
        assert!(p50[0] >= 49.0 && p50[0] <= 51.0);
    }

    #[test]
    fn group_by_count_distinct() {
        let batch = make_str_batch();
        let ops = vec![ViewOp::GroupBy {
            keys: vec![],
            aggs: vec![AggExpr::CountDistinct {
                column: "name".into(),
                alias: "cd".into(),
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let arr_idx = result[0].schema().index_of("cd").unwrap();
        let arr = result[0]
            .column(arr_idx)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(arr.value(0), 3);
    }

    #[test]
    fn group_by_no_keys() {
        let batch = make_int_batch(5);
        let ops = vec![ViewOp::GroupBy {
            keys: vec![],
            aggs: vec![AggExpr::Count {
                alias: "cnt".into(),
            }],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let arr_idx = result[0].schema().index_of("cnt").unwrap();
        let arr = result[0]
            .column(arr_idx)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(arr.value(0), 5);
    }

    #[test]
    fn derive_column_ref() {
        let batch = make_int_batch(3);
        let ops = vec![ViewOp::Derive {
            name: "id_copy".into(),
            expr: ScalarExpr::Column { name: "id".into() },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert!(result[0].schema().index_of("id_copy").is_ok());
    }

    #[test]
    fn derive_literal() {
        let batch = make_int_batch(3);
        let ops = vec![ViewOp::Derive {
            name: "const".into(),
            expr: ScalarExpr::Literal {
                value: Literal::Int(42),
            },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let idx = result[0].schema().index_of("const").unwrap();
        let arr = result[0]
            .column(idx)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        for i in 0..arr.len() {
            assert_eq!(arr.value(i), 42);
        }
    }

    #[test]
    fn derive_bin_op_add() {
        let batch = make_int_batch(4);
        let ops = vec![ViewOp::Derive {
            name: "doubled".into(),
            expr: ScalarExpr::BinOp {
                op: BinOp::Add,
                left: Box::new(ScalarExpr::Column { name: "id".into() }),
                right: Box::new(ScalarExpr::Column { name: "id".into() }),
            },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let idx = result[0].schema().index_of("doubled").unwrap();
        let arr = result[0]
            .column(idx)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        for i in 0..arr.len() {
            assert!((arr.value(i) - (i as f64 * 2.0)).abs() < 1e-9);
        }
    }

    #[test]
    fn derive_bin_op_div_by_zero() {
        let batch = make_int_batch(3);
        let ops = vec![ViewOp::Derive {
            name: "nan_col".into(),
            expr: ScalarExpr::BinOp {
                op: BinOp::Div,
                left: Box::new(ScalarExpr::Column { name: "id".into() }),
                right: Box::new(ScalarExpr::Literal {
                    value: Literal::Float(0.0),
                }),
            },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let idx = result[0].schema().index_of("nan_col").unwrap();
        let arr = result[0]
            .column(idx)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!(arr.value(1).is_nan());
    }

    #[test]
    fn derive_abs() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            ArrowDataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(vec![-3.0f64, 1.0, -5.0]))],
        )
        .unwrap();
        let ops = vec![ViewOp::Derive {
            name: "absx".into(),
            expr: ScalarExpr::Abs(Box::new(ScalarExpr::Column { name: "x".into() })),
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let idx = result[0].schema().index_of("absx").unwrap();
        let arr = result[0]
            .column(idx)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((arr.value(0) - 3.0).abs() < 1e-9);
        assert!((arr.value(2) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn derive_round() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            ArrowDataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(vec![3.567f64, 1.234]))],
        )
        .unwrap();
        let ops = vec![ViewOp::Derive {
            name: "rounded".into(),
            expr: ScalarExpr::Round {
                expr: Box::new(ScalarExpr::Column { name: "x".into() }),
                decimals: 2,
            },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let idx = result[0].schema().index_of("rounded").unwrap();
        let arr = result[0]
            .column(idx)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((arr.value(0) - 3.57).abs() < 1e-9);
        assert!((arr.value(1) - 1.23).abs() < 1e-9);
    }

    #[test]
    fn derive_floor_ceil() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            ArrowDataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Float64Array::from(vec![2.3f64, 2.7]))],
        )
        .unwrap();
        let ops_floor = vec![ViewOp::Derive {
            name: "fl".into(),
            expr: ScalarExpr::Floor(Box::new(ScalarExpr::Column { name: "x".into() })),
        }];
        let r1 = execute_pipeline(vec![batch.clone()], &ops_floor).unwrap();
        let idx = r1[0].schema().index_of("fl").unwrap();
        let arr = r1[0]
            .column(idx)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((arr.value(0) - 2.0).abs() < 1e-9);

        let ops_ceil = vec![ViewOp::Derive {
            name: "ce".into(),
            expr: ScalarExpr::Ceil(Box::new(ScalarExpr::Column { name: "x".into() })),
        }];
        let r2 = execute_pipeline(vec![batch], &ops_ceil).unwrap();
        let idx2 = r2[0].schema().index_of("ce").unwrap();
        let arr2 = r2[0]
            .column(idx2)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((arr2.value(0) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn derive_upper_lower_trim() {
        let batch = make_str_batch();
        let ops = vec![ViewOp::Derive {
            name: "upper_name".into(),
            expr: ScalarExpr::Upper(Box::new(ScalarExpr::Column {
                name: "name".into(),
            })),
        }];
        let result = execute_pipeline(vec![batch.clone()], &ops).unwrap();
        let idx = result[0].schema().index_of("upper_name").unwrap();
        let arr = result[0]
            .column(idx)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(arr.value(0), "ALICE");

        let schema = Arc::new(Schema::new(vec![Field::new(
            "s",
            ArrowDataType::Utf8,
            false,
        )]));
        let b2 = RecordBatch::try_new(schema, vec![Arc::new(StringArray::from(vec!["  hello  "]))])
            .unwrap();
        let ops2 = vec![ViewOp::Derive {
            name: "trimmed".into(),
            expr: ScalarExpr::Trim(Box::new(ScalarExpr::Column { name: "s".into() })),
        }];
        let r2 = execute_pipeline(vec![b2], &ops2).unwrap();
        let idx2 = r2[0].schema().index_of("trimmed").unwrap();
        let arr2 = r2[0]
            .column(idx2)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(arr2.value(0), "hello");
    }

    #[test]
    fn derive_length() {
        let batch = make_str_batch();
        let ops = vec![ViewOp::Derive {
            name: "len".into(),
            expr: ScalarExpr::Length(Box::new(ScalarExpr::Column {
                name: "name".into(),
            })),
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let idx = result[0].schema().index_of("len").unwrap();
        let arr = result[0]
            .column(idx)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(arr.value(0), 5);
        assert_eq!(arr.value(1), 3);
    }

    #[test]
    fn derive_substr() {
        let batch = make_str_batch();
        let ops = vec![ViewOp::Derive {
            name: "sub".into(),
            expr: ScalarExpr::Substr {
                expr: Box::new(ScalarExpr::Column {
                    name: "name".into(),
                }),
                start: 0,
                len: Some(3),
            },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let idx = result[0].schema().index_of("sub").unwrap();
        let arr = result[0]
            .column(idx)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(arr.value(0), "ali");
        assert_eq!(arr.value(1), "bob");
    }

    #[test]
    fn derive_concat() {
        let batch = make_str_batch();
        let ops = vec![ViewOp::Derive {
            name: "full".into(),
            expr: ScalarExpr::Concat {
                parts: vec![
                    ScalarExpr::Column {
                        name: "name".into(),
                    },
                    ScalarExpr::Literal {
                        value: Literal::Text("!".into()),
                    },
                ],
            },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let idx = result[0].schema().index_of("full").unwrap();
        let arr = result[0]
            .column(idx)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(arr.value(0), "alice!");
    }

    #[test]
    fn derive_cast_int_to_float() {
        let batch = make_int_batch(3);
        let ops = vec![ViewOp::Derive {
            name: "id_f".into(),
            expr: ScalarExpr::Cast {
                expr: Box::new(ScalarExpr::Column { name: "id".into() }),
                to_type: DataType::Float64,
            },
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        let idx = result[0].schema().index_of("id_f").unwrap();
        let arr = result[0]
            .column(idx)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((arr.value(0) - 0.0).abs() < 1e-9);
        assert!((arr.value(1) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn select_keeps_named_columns() {
        let batch = make_int_batch(3);
        let ops = vec![ViewOp::Select {
            columns: vec!["id".into()],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(result[0].num_columns(), 1);
        assert!(result[0].schema().index_of("id").is_ok());
    }

    #[test]
    fn select_missing_column_ignored() {
        let batch = make_int_batch(3);
        let ops = vec![ViewOp::Select {
            columns: vec!["id".into(), "nonexistent".into()],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(result[0].num_columns(), 1);
    }

    #[test]
    fn drop_removes_named_columns() {
        let batch = make_int_batch(3);
        let ops = vec![ViewOp::Drop {
            columns: vec!["score".into()],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(result[0].num_columns(), 1);
        assert!(result[0].schema().index_of("id").is_ok());
        assert!(result[0].schema().index_of("score").is_err());
    }

    #[test]
    fn deduplicate_all_columns() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            ArrowDataType::Int64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Int64Array::from(vec![1i64, 2, 2, 3, 1]))],
        )
        .unwrap();
        let ops = vec![ViewOp::Deduplicate { columns: None }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 3);
    }

    #[test]
    fn deduplicate_subset_columns() {
        let batch = make_str_batch();
        let ops = vec![ViewOp::Deduplicate {
            columns: Some(vec!["name".into()]),
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 3);
    }

    #[test]
    fn sample_deterministic_with_seed() {
        let batch = make_int_batch(100);
        let ops = vec![ViewOp::Sample {
            n: 10,
            strategy: tv_core::SampleStrategy::Bernoulli,
            seed: Some(42),
        }];
        let r1 = execute_pipeline(vec![batch.clone()], &ops).unwrap();
        let r2 = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&r1), 10);
        assert_eq!(total_rows(&r2), 10);
        let ids1: Vec<i64> = {
            let arr = r1[0]
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            (0..arr.len()).map(|i| arr.value(i)).collect()
        };
        let ids2: Vec<i64> = {
            let arr = r2[0]
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            (0..arr.len()).map(|i| arr.value(i)).collect()
        };
        assert_eq!(ids1, ids2);
    }

    #[test]
    fn sample_n_greater_than_total() {
        let batch = make_int_batch(5);
        let ops = vec![ViewOp::Sample {
            n: 100,
            strategy: tv_core::SampleStrategy::Bernoulli,
            seed: None,
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 5);
    }

    #[test]
    fn rename_changes_column_names() {
        let batch = make_int_batch(3);
        let ops = vec![ViewOp::Rename {
            mappings: vec![("id".into(), "row_id".into())],
        }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert!(result[0].schema().index_of("row_id").is_ok());
        assert!(result[0].schema().index_of("id").is_err());
    }

    #[test]
    fn limit_takes_first_n() {
        let batch = make_int_batch(10);
        let ops = vec![ViewOp::Limit { n: 3 }];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 3);
    }

    #[test]
    fn execute_pipeline_chained_ops() {
        let batch = make_int_batch(20);
        let ops = vec![
            ViewOp::Filter {
                predicate: Predicate::Lt {
                    column: "id".into(),
                    value: Literal::Int(10),
                },
            },
            ViewOp::Sort {
                keys: vec![SortKey {
                    column: "id".into(),
                    descending: true,
                    nulls_last: true,
                }],
            },
            ViewOp::Limit { n: 5 },
        ];
        let result = execute_pipeline(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 5);
        let arr = result[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(arr.value(0), 9);
    }

    #[test]
    fn execute_pipeline_skip_filter_test() {
        let batch = make_int_batch(10);
        let ops = vec![
            ViewOp::Filter {
                predicate: Predicate::Eq {
                    column: "id".into(),
                    value: Literal::Int(0),
                },
            },
            ViewOp::Limit { n: 3 },
        ];
        let result = execute_pipeline_skip_filter(vec![batch], &ops).unwrap();
        assert_eq!(total_rows(&result), 3);
    }
}
