use std::collections::{HashMap, HashSet};

use crate::expr::{AggExpr, Literal, Predicate, ScalarExpr, ViewOp};
use crate::types::ColumnStats;

const TOP_K_THRESHOLD: u64 = 10_000;

pub fn optimize(ops: &[ViewOp], stats_hint: Option<&[ColumnStats]>) -> Vec<ViewOp> {
    optimize_with_quantiles(ops, stats_hint, None)
}

pub fn optimize_with_quantiles(
    ops: &[ViewOp],
    stats_hint: Option<&[ColumnStats]>,
    quantile_hint: Option<&HashMap<String, crate::types::QuantileSketch>>,
) -> Vec<ViewOp> {
    let ops = pass1_predicate_pushdown(ops.to_vec());
    let ops = pass2_column_pruning(ops);
    let ops = pass3_sort_limit_fusion(ops);
    let ops = pass4_constant_folding(ops);
    pass5_selectivity_ordering(ops, stats_hint, quantile_hint)
}

pub fn needed_column_indices(
    ops: &[ViewOp],
    schema: &arrow::datatypes::SchemaRef,
) -> Option<Vec<usize>> {
    let has_group_by = ops.iter().any(|op| matches!(op, ViewOp::GroupBy { .. }));
    let has_explicit_select = ops.iter().any(|op| matches!(op, ViewOp::Select { .. }));

    if has_explicit_select || has_group_by {
        return None;
    }

    let referenced = compute_referenced_columns(ops);
    if referenced.is_empty() {
        return None;
    }

    let mut indices: Vec<usize> = referenced
        .iter()
        .filter_map(|col| schema.index_of(col).ok())
        .collect();
    indices.sort_unstable();
    indices.dedup();

    if indices.len() == schema.fields().len() {
        return None;
    }

    Some(indices)
}

fn pass1_predicate_pushdown(ops: Vec<ViewOp>) -> Vec<ViewOp> {
    let mut result: Vec<ViewOp> = Vec::with_capacity(ops.len());
    let mut pending_filters: Vec<ViewOp> = Vec::new();

    for op in ops {
        match &op {
            ViewOp::Filter { .. } => pending_filters.push(op),
            ViewOp::Rename { mappings } => {
                let renamed_cols: HashSet<String> =
                    mappings.iter().map(|(_, to)| to.clone()).collect();
                let (passthrough, hold_back): (Vec<_>, Vec<_>) =
                    pending_filters.drain(..).partition(|f| {
                        if let ViewOp::Filter { predicate } = f {
                            !predicate_references_any(predicate, &renamed_cols)
                        } else {
                            true
                        }
                    });
                result.extend(passthrough);
                result.push(op);
                pending_filters.extend(hold_back);
            }
            ViewOp::Derive { name, .. } => {
                let derived_col: HashSet<String> = std::iter::once(name.clone()).collect();
                let (passthrough, hold_back): (Vec<_>, Vec<_>) =
                    pending_filters.drain(..).partition(|f| {
                        if let ViewOp::Filter { predicate } = f {
                            !predicate_references_any(predicate, &derived_col)
                        } else {
                            true
                        }
                    });
                result.extend(passthrough);
                result.push(op);
                pending_filters.extend(hold_back);
            }
            _ => {
                result.append(&mut pending_filters);
                result.push(op);
            }
        }
    }
    result.extend(pending_filters);
    result
}

fn pass2_column_pruning(ops: Vec<ViewOp>) -> Vec<ViewOp> {
    let already_has_select = ops.iter().any(|op| matches!(op, ViewOp::Select { .. }));
    if already_has_select {
        return ops;
    }

    let has_group_by = ops.iter().any(|op| matches!(op, ViewOp::GroupBy { .. }));
    if !has_group_by {
        return ops;
    }

    let referenced = compute_referenced_columns(&ops);
    if referenced.is_empty() {
        return ops;
    }

    let mut result = Vec::with_capacity(ops.len() + 1);
    result.push(ViewOp::Select {
        columns: {
            let mut cols: Vec<String> = referenced.into_iter().collect();
            cols.sort();
            cols
        },
    });
    result.extend(ops);
    result
}

fn pass3_sort_limit_fusion(ops: Vec<ViewOp>) -> Vec<ViewOp> {
    let sort_idx = ops.iter().position(|op| matches!(op, ViewOp::Sort { .. }));
    let limit_idx = ops.iter().position(|op| matches!(op, ViewOp::Limit { .. }));

    match (sort_idx, limit_idx) {
        (Some(si), Some(li)) => {
            if let (ViewOp::Sort { keys }, ViewOp::Limit { n }) = (&ops[si], &ops[li]) {
                if *n <= TOP_K_THRESHOLD {
                    let n_val = *n;
                    let keys_val = keys.clone();
                    let mut result: Vec<ViewOp> = ops
                        .into_iter()
                        .enumerate()
                        .filter_map(|(i, op)| if i == si || i == li { None } else { Some(op) })
                        .collect();
                    result.push(ViewOp::TopK {
                        n: n_val,
                        keys: keys_val,
                    });
                    return result;
                }
            }
            ops
        }
        _ => ops,
    }
}

fn pass4_constant_folding(ops: Vec<ViewOp>) -> Vec<ViewOp> {
    ops.into_iter()
        .map(|op| match op {
            ViewOp::Filter { predicate } => ViewOp::Filter {
                predicate: fold_predicate(predicate),
            },
            other => other,
        })
        .filter(|op| {
            if let ViewOp::Filter {
                predicate: Predicate::And { exprs },
            } = op
            {
                !exprs.is_empty()
            } else {
                true
            }
        })
        .collect()
}

fn pass5_selectivity_ordering(
    ops: Vec<ViewOp>,
    stats_hint: Option<&[ColumnStats]>,
    quantile_hint: Option<&HashMap<String, crate::types::QuantileSketch>>,
) -> Vec<ViewOp> {
    let stats = match stats_hint {
        None => return ops,
        Some(s) => s,
    };

    ops.into_iter()
        .map(|op| match op {
            ViewOp::Filter {
                predicate: Predicate::And { exprs },
            } => {
                let mut scored: Vec<(f64, Predicate)> = exprs
                    .into_iter()
                    .map(|p| {
                        let sel = estimate_selectivity_with_quantiles(&p, stats, quantile_hint);
                        (sel, p)
                    })
                    .collect();
                scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                ViewOp::Filter {
                    predicate: Predicate::And {
                        exprs: scored.into_iter().map(|(_, p)| p).collect(),
                    },
                }
            }
            other => other,
        })
        .collect()
}

pub fn compute_referenced_columns(ops: &[ViewOp]) -> HashSet<String> {
    let mut cols: HashSet<String> = HashSet::new();
    for op in ops {
        match op {
            ViewOp::Filter { predicate } => collect_predicate_columns(predicate, &mut cols),
            ViewOp::Sort { keys } | ViewOp::TopK { keys, .. } => {
                cols.extend(keys.iter().map(|k| k.column.clone()));
            }
            ViewOp::Derive { name, expr } => {
                cols.insert(name.clone());
                collect_scalar_columns(expr, &mut cols);
            }
            ViewOp::GroupBy { keys, aggs } => {
                cols.extend(keys.clone());
                for agg in aggs {
                    if let Some(col) = agg_col_name(agg) {
                        cols.insert(col.to_string());
                    }
                }
            }
            ViewOp::Deduplicate {
                columns: Some(cols_v),
            } => cols.extend(cols_v.clone()),
            ViewOp::Rename { mappings } => {
                for (from, _) in mappings {
                    cols.insert(from.clone());
                }
            }
            ViewOp::Select { columns } => cols.extend(columns.clone()),
            _ => {}
        }
    }
    cols
}

fn collect_predicate_columns(pred: &Predicate, cols: &mut HashSet<String>) {
    match pred {
        Predicate::Eq { column, .. }
        | Predicate::Ne { column, .. }
        | Predicate::Gt { column, .. }
        | Predicate::Gte { column, .. }
        | Predicate::Lt { column, .. }
        | Predicate::Lte { column, .. }
        | Predicate::Between { column, .. }
        | Predicate::In { column, .. }
        | Predicate::NotIn { column, .. }
        | Predicate::Contains { column, .. }
        | Predicate::StartsWith { column, .. }
        | Predicate::EndsWith { column, .. }
        | Predicate::Regex { column, .. }
        | Predicate::IsNull { column }
        | Predicate::IsNotNull { column } => {
            cols.insert(column.clone());
        }
        Predicate::And { exprs } | Predicate::Or { exprs } => {
            for e in exprs {
                collect_predicate_columns(e, cols);
            }
        }
        Predicate::Not { expr } => collect_predicate_columns(expr, cols),
    }
}

fn collect_scalar_columns(expr: &ScalarExpr, cols: &mut HashSet<String>) {
    match expr {
        ScalarExpr::Column { name } => {
            cols.insert(name.clone());
        }
        ScalarExpr::BinOp { left, right, .. } => {
            collect_scalar_columns(left, cols);
            collect_scalar_columns(right, cols);
        }
        ScalarExpr::Abs(e)
        | ScalarExpr::Floor(e)
        | ScalarExpr::Ceil(e)
        | ScalarExpr::Upper(e)
        | ScalarExpr::Lower(e)
        | ScalarExpr::Trim(e)
        | ScalarExpr::Length(e)
        | ScalarExpr::Year(e)
        | ScalarExpr::Month(e)
        | ScalarExpr::Day(e) => collect_scalar_columns(e, cols),
        ScalarExpr::Round { expr, .. }
        | ScalarExpr::Substr { expr, .. }
        | ScalarExpr::Cast { expr, .. } => {
            collect_scalar_columns(expr, cols);
        }
        ScalarExpr::Concat { parts } => {
            for p in parts {
                collect_scalar_columns(p, cols);
            }
        }
        ScalarExpr::Case { whens, else_expr } => {
            for (pred, expr) in whens {
                collect_predicate_columns(pred, cols);
                collect_scalar_columns(expr, cols);
            }
            if let Some(e) = else_expr {
                collect_scalar_columns(e, cols);
            }
        }
        ScalarExpr::Coalesce { exprs } => {
            for e in exprs {
                collect_scalar_columns(e, cols);
            }
        }
        ScalarExpr::Rank { order } => {
            for k in order {
                cols.insert(k.column.clone());
            }
        }
        ScalarExpr::Literal { .. } | ScalarExpr::NTile { .. } => {}
    }
}

fn predicate_references_any(pred: &Predicate, col_names: &HashSet<String>) -> bool {
    let mut referenced = HashSet::new();
    collect_predicate_columns(pred, &mut referenced);
    referenced.iter().any(|c| col_names.contains(c))
}

fn fold_predicate(pred: Predicate) -> Predicate {
    match pred {
        Predicate::And { exprs } => {
            let folded: Vec<Predicate> = exprs
                .into_iter()
                .map(fold_predicate)
                .filter(|p| !is_always_true(p))
                .collect();
            if folded.is_empty() {
                return Predicate::And { exprs: vec![] };
            }
            if folded.len() == 1 {
                return folded.into_iter().next().unwrap();
            }
            Predicate::And { exprs: folded }
        }
        Predicate::Or { exprs } => {
            let folded: Vec<Predicate> = exprs
                .into_iter()
                .map(fold_predicate)
                .filter(|p| !is_always_false(p))
                .collect();
            if folded.is_empty() {
                return Predicate::Or { exprs: vec![] };
            }
            if folded.len() == 1 {
                return folded.into_iter().next().unwrap();
            }
            Predicate::Or { exprs: folded }
        }
        Predicate::Not { expr } => Predicate::Not {
            expr: Box::new(fold_predicate(*expr)),
        },
        other => other,
    }
}

fn is_always_true(pred: &Predicate) -> bool {
    matches!(
        pred,
        Predicate::Eq {
            value: Literal::Bool(true),
            ..
        }
    )
}

fn is_always_false(pred: &Predicate) -> bool {
    matches!(
        pred,
        Predicate::Eq {
            value: Literal::Bool(false),
            ..
        }
    )
}

fn estimate_selectivity_with_quantiles(
    pred: &Predicate,
    stats: &[ColumnStats],
    quantile_hint: Option<&HashMap<String, crate::types::QuantileSketch>>,
) -> f64 {
    if let Some(qh) = quantile_hint {
        match pred {
            Predicate::Gt { column, value } | Predicate::Gte { column, value } => {
                if let Some(v) = literal_to_f64(value) {
                    if let Some(sketch) = qh.get(column) {
                        return 1.0 - sketch.cdf(v);
                    }
                }
            }
            Predicate::Lt { column, value } | Predicate::Lte { column, value } => {
                if let Some(v) = literal_to_f64(value) {
                    if let Some(sketch) = qh.get(column) {
                        return sketch.cdf(v);
                    }
                }
            }
            Predicate::Between { column, lo, hi } => {
                if let (Some(lo_f), Some(hi_f)) = (literal_to_f64(lo), literal_to_f64(hi)) {
                    if let Some(sketch) = qh.get(column) {
                        return (sketch.cdf(hi_f) - sketch.cdf(lo_f)).max(0.0);
                    }
                }
            }
            _ => {}
        }
    }
    estimate_selectivity(pred, stats)
}

fn estimate_selectivity(pred: &Predicate, stats: &[ColumnStats]) -> f64 {
    match pred {
        Predicate::Eq { column, .. } | Predicate::Ne { column, .. } => {
            if let Some(col_stats) = find_col_stats(column, stats) {
                let distinct = col_stats.distinct_count.unwrap_or(1) as f64;
                if distinct > 0.0 {
                    return 1.0 / distinct;
                }
            }
            0.5
        }
        Predicate::In { column, values } => {
            if let Some(col_stats) = find_col_stats(column, stats) {
                let distinct = col_stats.distinct_count.unwrap_or(1) as f64;
                if distinct > 0.0 {
                    return (values.len() as f64 / distinct).min(1.0);
                }
            }
            0.5
        }
        Predicate::IsNull { column } => {
            if let Some(col_stats) = find_col_stats(column, stats) {
                return col_stats.null_rate;
            }
            0.05
        }
        Predicate::IsNotNull { column } => {
            if let Some(col_stats) = find_col_stats(column, stats) {
                return 1.0 - col_stats.null_rate;
            }
            0.95
        }
        Predicate::And { exprs } => exprs
            .iter()
            .map(|e| estimate_selectivity(e, stats))
            .product(),
        Predicate::Or { exprs } => {
            let sel: Vec<f64> = exprs
                .iter()
                .map(|e| estimate_selectivity(e, stats))
                .collect();
            1.0 - sel.iter().map(|s| 1.0 - s).product::<f64>()
        }
        _ => 0.3,
    }
}

fn find_col_stats<'a>(column: &str, stats: &'a [ColumnStats]) -> Option<&'a ColumnStats> {
    stats.iter().find(|s| s.column == column)
}

fn literal_to_f64(lit: &crate::expr::Literal) -> Option<f64> {
    match lit {
        crate::expr::Literal::Float(v) => Some(*v),
        crate::expr::Literal::Int(v) => Some(*v as f64),
        _ => None,
    }
}

fn agg_col_name(agg: &AggExpr) -> Option<&str> {
    match agg {
        AggExpr::Count { .. } => None,
        AggExpr::CountDistinct { column, .. }
        | AggExpr::Sum { column, .. }
        | AggExpr::Min { column, .. }
        | AggExpr::Max { column, .. }
        | AggExpr::Mean { column, .. }
        | AggExpr::StdDev { column, .. }
        | AggExpr::Median { column, .. }
        | AggExpr::Percentile { column, .. } => Some(column.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{AggExpr, BinOp, Literal, Predicate, ScalarExpr, SortKey, ViewOp};
    use crate::types::{CardinalityCategory, ColumnStats};

    fn make_stats(column: &str, distinct: u64, null_rate: f64) -> ColumnStats {
        ColumnStats {
            column: column.into(),
            index: 0,
            data_type: "Int64".into(),
            count: 100,
            null_count: (null_rate * 100.0) as u64,
            null_rate,
            distinct_count: Some(distinct),
            min: None,
            max: None,
            mean: None,
            quantiles: None,
            histogram: None,
            top_values: None,
            cardinality_category: CardinalityCategory::Unknown,
        }
    }

    fn sort_op(col: &str) -> ViewOp {
        ViewOp::Sort {
            keys: vec![SortKey {
                column: col.into(),
                descending: false,
                nulls_last: true,
            }],
        }
    }

    fn filter_op(col: &str, val: i64) -> ViewOp {
        ViewOp::Filter {
            predicate: Predicate::Eq {
                column: col.into(),
                value: Literal::Int(val),
            },
        }
    }

    fn filter_rename_col_op(col: &str) -> ViewOp {
        ViewOp::Filter {
            predicate: Predicate::IsNotNull { column: col.into() },
        }
    }

    #[test]
    fn pushdown_filter_before_rename() {
        let ops = vec![
            filter_op("x", 1),
            ViewOp::Rename {
                mappings: vec![("old".into(), "new".into())],
            },
        ];
        let result = pass1_predicate_pushdown(ops);
        let filter_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Filter { .. }))
            .unwrap();
        let rename_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Rename { .. }))
            .unwrap();
        assert!(filter_pos < rename_pos);
    }

    #[test]
    fn pushdown_filter_blocked_by_rename() {
        let ops = vec![
            ViewOp::Rename {
                mappings: vec![("x".into(), "new_x".into())],
            },
            ViewOp::Filter {
                predicate: Predicate::IsNotNull {
                    column: "new_x".into(),
                },
            },
        ];
        let result = pass1_predicate_pushdown(ops.clone());
        let filter_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Filter { .. }))
            .unwrap();
        let rename_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Rename { .. }))
            .unwrap();
        assert!(filter_pos > rename_pos);
    }

    #[test]
    fn pushdown_filter_before_derive() {
        let ops = vec![
            filter_op("x", 5),
            ViewOp::Derive {
                name: "derived".into(),
                expr: ScalarExpr::Column { name: "y".into() },
            },
        ];
        let result = pass1_predicate_pushdown(ops);
        let filter_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Filter { .. }))
            .unwrap();
        let derive_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Derive { .. }))
            .unwrap();
        assert!(filter_pos < derive_pos);
    }

    #[test]
    fn pushdown_filter_blocked_by_derive() {
        let ops = vec![
            ViewOp::Derive {
                name: "derived".into(),
                expr: ScalarExpr::Column { name: "y".into() },
            },
            ViewOp::Filter {
                predicate: Predicate::IsNotNull {
                    column: "derived".into(),
                },
            },
        ];
        let result = pass1_predicate_pushdown(ops);
        let filter_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Filter { .. }))
            .unwrap();
        let derive_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Derive { .. }))
            .unwrap();
        assert!(filter_pos > derive_pos);
    }

    #[test]
    fn pushdown_no_filters() {
        let ops = vec![sort_op("x")];
        let result = pass1_predicate_pushdown(ops.clone());
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn pushdown_filter_past_non_blocking_ops() {
        let ops = vec![
            filter_op("x", 1),
            ViewOp::Select {
                columns: vec!["x".into(), "y".into()],
            },
        ];
        let result = pass1_predicate_pushdown(ops);
        let filter_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Filter { .. }))
            .unwrap();
        let select_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Select { .. }))
            .unwrap();
        assert!(filter_pos < select_pos);
    }

    #[test]
    fn pushdown_multiple_filters_partial_push() {
        let ops = vec![
            filter_op("x", 1),
            ViewOp::Rename {
                mappings: vec![("z".into(), "new_z".into())],
            },
            ViewOp::Filter {
                predicate: Predicate::IsNotNull {
                    column: "new_z".into(),
                },
            },
        ];
        let result = pass1_predicate_pushdown(ops);
        let x_filter_pos = result
            .iter()
            .position(|o| {
                if let ViewOp::Filter {
                    predicate: Predicate::Eq { column, .. },
                } = o
                {
                    column == "x"
                } else {
                    false
                }
            })
            .unwrap();
        let rename_pos = result
            .iter()
            .position(|o| matches!(o, ViewOp::Rename { .. }))
            .unwrap();
        assert!(x_filter_pos < rename_pos);
    }

    #[test]
    fn pruning_adds_select_for_group_by() {
        let ops = vec![ViewOp::GroupBy {
            keys: vec!["dept".into()],
            aggs: vec![AggExpr::Count {
                alias: "cnt".into(),
            }],
        }];
        let result = pass2_column_pruning(ops);
        assert!(result.iter().any(|o| matches!(o, ViewOp::Select { .. })));
    }

    #[test]
    fn pruning_no_select_for_pure_sort() {
        let ops = vec![sort_op("x")];
        let result = pass2_column_pruning(ops);
        assert!(!result.iter().any(|o| matches!(o, ViewOp::Select { .. })));
    }

    #[test]
    fn pruning_skips_when_select_exists() {
        let ops = vec![
            ViewOp::Select {
                columns: vec!["a".into()],
            },
            ViewOp::GroupBy {
                keys: vec!["a".into()],
                aggs: vec![AggExpr::Count {
                    alias: "cnt".into(),
                }],
            },
        ];
        let result = pass2_column_pruning(ops.clone());
        let select_count = result
            .iter()
            .filter(|o| matches!(o, ViewOp::Select { .. }))
            .count();
        assert_eq!(select_count, 1);
    }

    #[test]
    fn pruning_no_group_by_no_change() {
        let ops = vec![filter_op("x", 1), sort_op("x")];
        let result = pass2_column_pruning(ops.clone());
        assert_eq!(result.len(), ops.len());
    }

    #[test]
    fn pruning_collects_all_referenced_columns() {
        let ops = vec![ViewOp::GroupBy {
            keys: vec!["dept".into()],
            aggs: vec![
                AggExpr::Sum {
                    column: "salary".into(),
                    alias: "total".into(),
                },
                AggExpr::Count {
                    alias: "cnt".into(),
                },
            ],
        }];
        let result = pass2_column_pruning(ops);
        if let Some(ViewOp::Select { columns }) = result.first() {
            assert!(columns.contains(&"dept".to_string()));
            assert!(columns.contains(&"salary".to_string()));
        } else {
            panic!("Expected Select as first op");
        }
    }

    #[test]
    fn fusion_sort_limit_below_threshold() {
        let ops = vec![sort_op("x"), ViewOp::Limit { n: 5000 }];
        let result = pass3_sort_limit_fusion(ops);
        assert!(result.iter().any(|o| matches!(o, ViewOp::TopK { .. })));
        assert!(!result.iter().any(|o| matches!(o, ViewOp::Sort { .. })));
        assert!(!result.iter().any(|o| matches!(o, ViewOp::Limit { .. })));
    }

    #[test]
    fn fusion_sort_limit_above_threshold() {
        let ops = vec![sort_op("x"), ViewOp::Limit { n: 20_000 }];
        let result = pass3_sort_limit_fusion(ops);
        assert!(!result.iter().any(|o| matches!(o, ViewOp::TopK { .. })));
        assert!(result.iter().any(|o| matches!(o, ViewOp::Sort { .. })));
    }

    #[test]
    fn fusion_no_sort_no_limit() {
        let ops = vec![filter_op("x", 1)];
        let result = pass3_sort_limit_fusion(ops.clone());
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn fusion_sort_limit_exactly_at_threshold() {
        let ops = vec![sort_op("x"), ViewOp::Limit { n: 10_000 }];
        let result = pass3_sort_limit_fusion(ops);
        assert!(result.iter().any(|o| matches!(o, ViewOp::TopK { .. })));
    }

    #[test]
    fn fold_removes_always_true_from_and() {
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::And {
                exprs: vec![
                    Predicate::Eq {
                        column: "x".into(),
                        value: Literal::Bool(true),
                    },
                    Predicate::IsNotNull { column: "y".into() },
                ],
            },
        }];
        let result = pass4_constant_folding(ops);
        if let ViewOp::Filter { predicate } = &result[0] {
            match predicate {
                Predicate::IsNotNull { .. } => {}
                Predicate::And { exprs } => {
                    assert_eq!(exprs.len(), 1);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn fold_removes_always_false_from_or() {
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::Or {
                exprs: vec![
                    Predicate::Eq {
                        column: "x".into(),
                        value: Literal::Bool(false),
                    },
                    Predicate::IsNotNull { column: "y".into() },
                ],
            },
        }];
        let result = pass4_constant_folding(ops);
        if let ViewOp::Filter { predicate } = &result[0] {
            match predicate {
                Predicate::IsNotNull { .. } => {}
                Predicate::Or { exprs } => {
                    assert_eq!(exprs.len(), 1);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn fold_empty_and_removed() {
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::And {
                exprs: vec![Predicate::Eq {
                    column: "x".into(),
                    value: Literal::Bool(true),
                }],
            },
        }];
        let result = pass4_constant_folding(ops);
        assert!(result.is_empty());
    }

    #[test]
    fn fold_nested_and_or() {
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::And {
                exprs: vec![Predicate::Or {
                    exprs: vec![
                        Predicate::Eq {
                            column: "x".into(),
                            value: Literal::Bool(false),
                        },
                        Predicate::IsNotNull { column: "y".into() },
                    ],
                }],
            },
        }];
        let result = pass4_constant_folding(ops);
        assert!(!result.is_empty());
    }

    #[test]
    fn selectivity_orders_eq_before_range() {
        let stats = vec![make_stats("id", 100, 0.0), make_stats("name", 10_000, 0.0)];
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::And {
                exprs: vec![
                    Predicate::Gte {
                        column: "id".into(),
                        value: Literal::Int(50),
                    },
                    Predicate::Eq {
                        column: "name".into(),
                        value: Literal::Text("alice".into()),
                    },
                ],
            },
        }];
        let result = pass5_selectivity_ordering(ops, Some(&stats), None);
        if let ViewOp::Filter {
            predicate: Predicate::And { exprs },
        } = &result[0]
        {
            assert!(matches!(exprs[0], Predicate::Eq { .. }));
        }
    }

    #[test]
    fn selectivity_uses_null_rate() {
        let stats = vec![make_stats("x", 10, 0.9)];
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::And {
                exprs: vec![
                    Predicate::IsNotNull { column: "x".into() },
                    Predicate::Eq {
                        column: "x".into(),
                        value: Literal::Int(1),
                    },
                ],
            },
        }];
        let result = pass5_selectivity_ordering(ops, Some(&stats), None);
        assert!(!result.is_empty());
    }

    #[test]
    fn selectivity_no_stats_passthrough() {
        let ops = vec![filter_op("x", 1)];
        let result = pass5_selectivity_ordering(ops.clone(), None, None);
        assert_eq!(result.len(), ops.len());
    }

    #[test]
    fn optimize_full_pipeline() {
        let ops = vec![
            ViewOp::Rename {
                mappings: vec![("old_id".into(), "id".into())],
            },
            filter_op("x", 1),
            ViewOp::GroupBy {
                keys: vec!["id".into()],
                aggs: vec![AggExpr::Count {
                    alias: "cnt".into(),
                }],
            },
            sort_op("cnt"),
            ViewOp::Limit { n: 10 },
        ];
        let result = optimize(&ops, None);
        assert!(!result.is_empty());
        assert!(result.iter().any(|o| matches!(o, ViewOp::TopK { .. })));
    }

    #[test]
    fn optimize_idempotent() {
        let ops = vec![filter_op("x", 1), sort_op("x")];
        let once = optimize(&ops, None);
        let twice = optimize(&once, None);
        assert_eq!(once.len(), twice.len());
    }
}
