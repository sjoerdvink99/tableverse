use crate::compiler::agg::agg_to_sql;
use crate::compiler::predicate::{esc, predicate_to_sql};
use crate::compiler::reorder_sort_last;
use crate::compiler::scalar::scalar_to_sql;
use tv_core::{AggExpr, Literal, Predicate, ScalarExpr, SortKey, SourceFormat, ViewExpr, ViewOp};

pub fn render_sql(expr: &ViewExpr, uri: &str, format: &SourceFormat) -> String {
    let read_expr = read_fn(uri, format);
    let ops = reorder_sort_last(&expr.ops);

    let predicates: Vec<&Predicate> = ops
        .iter()
        .filter_map(|op| {
            if let ViewOp::Filter { predicate } = op {
                Some(predicate)
            } else {
                None
            }
        })
        .collect();

    let last_select: Option<&Vec<String>> = ops.iter().rev().find_map(|op| {
        if let ViewOp::Select { columns } = op {
            Some(columns)
        } else {
            None
        }
    });

    let drops: Vec<&Vec<String>> = ops
        .iter()
        .filter_map(|op| {
            if let ViewOp::Drop { columns } = op {
                Some(columns)
            } else {
                None
            }
        })
        .collect();

    let derives: Vec<(&str, &ScalarExpr)> = ops
        .iter()
        .filter_map(|op| {
            if let ViewOp::Derive { name, expr } = op {
                Some((name.as_str(), expr))
            } else {
                None
            }
        })
        .collect();

    let renames: Vec<&Vec<(String, String)>> = ops
        .iter()
        .filter_map(|op| {
            if let ViewOp::Rename { mappings } = op {
                Some(mappings)
            } else {
                None
            }
        })
        .collect();

    let group_by: Option<(&Vec<String>, &Vec<AggExpr>)> = ops.iter().find_map(|op| {
        if let ViewOp::GroupBy { keys, aggs } = op {
            Some((keys, aggs))
        } else {
            None
        }
    });

    let sort: Option<&Vec<SortKey>> = ops.iter().rev().find_map(|op| {
        if let ViewOp::Sort { keys } = op {
            Some(keys)
        } else {
            None
        }
    });

    let sample: Option<&ViewOp> = ops.iter().find(|op| matches!(op, ViewOp::Sample { .. }));

    let limit: Option<u64> = ops.iter().rev().find_map(|op| {
        if let ViewOp::Limit { n } = op {
            Some(*n)
        } else {
            None
        }
    });

    let select_clause = if let Some((keys, aggs)) = group_by {
        let mut parts: Vec<String> = keys.iter().map(|k| format!("\"{}\"", esc(k))).collect();
        parts.extend(aggs.iter().map(agg_to_sql));
        parts.join(",\n    ")
    } else {
        build_select(last_select, &drops, &derives, &renames)
    };

    let where_clause = if predicates.is_empty() {
        String::new()
    } else {
        let combined: Vec<String> = predicates.iter().map(|p| predicate_to_sql(p)).collect();
        format!("\nWHERE {}", combined.join("\n  AND "))
    };

    let sample_clause = if let Some(ViewOp::Sample { n, strategy, seed }) = sample {
        let method = match strategy {
            tv_core::SampleStrategy::Bernoulli => "bernoulli",
            tv_core::SampleStrategy::System => "system",
        };
        let seed_part = seed.map(|s| format!(", {s}")).unwrap_or_default();
        format!("\nUSING SAMPLE {n} ROWS ({method}{seed_part})")
    } else {
        String::new()
    };

    let group_by_clause = if let Some((keys, _)) = group_by {
        let cols: Vec<String> = keys.iter().map(|k| format!("\"{}\"", esc(k))).collect();
        format!("\nGROUP BY {}", cols.join(", "))
    } else {
        String::new()
    };

    let order_by_clause = if let Some(keys) = sort {
        let parts: Vec<String> = keys
            .iter()
            .map(|k| {
                let col = esc(&k.column);
                let dir = if k.descending { "DESC" } else { "ASC" };
                let nulls = if k.nulls_last {
                    "NULLS LAST"
                } else {
                    "NULLS FIRST"
                };
                format!("\"{col}\" {dir} {nulls}")
            })
            .collect();
        format!("\nORDER BY {}", parts.join(", "))
    } else {
        String::new()
    };

    let limit_clause = match limit {
        Some(n) => format!("\nLIMIT {n}"),
        None => String::new(),
    };

    format!(
        "SELECT {select_clause}\nFROM {read_expr}{sample_clause}{where_clause}{group_by_clause}{order_by_clause}{limit_clause}"
    )
}

fn build_select(
    last_select: Option<&Vec<String>>,
    drops: &[&Vec<String>],
    derives: &[(&str, &ScalarExpr)],
    renames: &[&Vec<(String, String)>],
) -> String {
    let flat_renames: Vec<&(String, String)> = renames.iter().flat_map(|m| m.iter()).collect();

    let mut parts: Vec<String> = if let Some(cols) = last_select {
        cols.iter()
            .map(|c| {
                let new_name = flat_renames
                    .iter()
                    .find(|(from, _)| from == c)
                    .map(|(_, to)| to.as_str())
                    .unwrap_or(c.as_str());
                if new_name == c.as_str() {
                    format!("\"{}\"", esc(c))
                } else {
                    format!("\"{}\" AS \"{}\"", esc(c), esc(new_name))
                }
            })
            .collect()
    } else if !drops.is_empty() {
        let excluded: Vec<String> = drops
            .iter()
            .flat_map(|d| d.iter())
            .map(|c| format!("\"{}\"", esc(c)))
            .collect();
        vec![format!("* EXCLUDE ({})", excluded.join(", "))]
    } else if !flat_renames.is_empty() {
        let rename_exprs: Vec<String> = flat_renames
            .iter()
            .map(|(from, to)| format!("\"{}\" AS \"{}\"", esc(from), esc(to)))
            .collect();
        let mut result = vec!["*".to_string()];
        result.extend(rename_exprs);
        result
    } else {
        vec!["*".to_string()]
    };

    for (name, expr) in derives {
        parts.push(format!("{} AS \"{}\"", scalar_to_sql(expr), esc(name)));
    }

    parts.join(",\n    ")
}

fn read_fn(uri: &str, format: &SourceFormat) -> String {
    let escaped = uri.replace('\'', "''");
    match format {
        SourceFormat::Parquet => format!("read_parquet('{escaped}')"),
        SourceFormat::Csv => format!("read_csv_auto('{escaped}')"),
        SourceFormat::Arrow => format!("read_parquet('{escaped}')"),
        SourceFormat::Json => format!("read_json_auto('{escaped}')"),
        SourceFormat::Delta => format!("delta_scan('{escaped}')"),
        SourceFormat::Iceberg => format!("iceberg_scan('{escaped}')"),
        SourceFormat::Database => format!("'{escaped}'"),
    }
}

pub fn lit_sql(value: &Literal) -> String {
    crate::compiler::predicate::lit(value)
}
