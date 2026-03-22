use crate::compiler::agg::agg_to_sql;
use crate::compiler::predicate::{esc, predicate_to_sql};
use crate::compiler::reorder_sort_last;
use crate::compiler::scalar::scalar_to_sql;
use tv_core::{AggExpr, Predicate, ScalarExpr, SortKey, ViewExpr, ViewOp};

pub fn render_ansi_sql(expr: &ViewExpr, table_name: &str) -> String {
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
    let flat_renames: Vec<&(String, String)> = renames.iter().flat_map(|m| m.iter()).collect();

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
    } else if let Some(cols) = last_select {
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
            .collect::<Vec<_>>()
            .join(",\n    ")
    } else if !drops.is_empty() {
        "*".to_string()
    } else if !flat_renames.is_empty() {
        let rename_exprs: Vec<String> = flat_renames
            .iter()
            .map(|(from, to)| format!("\"{}\" AS \"{}\"", esc(from), esc(to)))
            .collect();
        let mut parts = vec!["*".to_string()];
        parts.extend(rename_exprs);
        parts.join(",\n    ")
    } else {
        let mut parts = vec!["*".to_string()];
        for (name, expr) in &derives {
            parts.push(format!("{} AS \"{}\"", scalar_to_sql(expr), esc(name)));
        }
        parts.join(",\n    ")
    };

    let where_clause = if predicates.is_empty() {
        String::new()
    } else {
        let combined: Vec<String> = predicates.iter().map(|p| predicate_to_sql(p)).collect();
        format!("\nWHERE {}", combined.join("\n  AND "))
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
                format!("\"{col}\" {dir}")
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
        "SELECT {select_clause}\nFROM \"{}\"{where_clause}{group_by_clause}{order_by_clause}{limit_clause}",
        esc(table_name)
    )
}
