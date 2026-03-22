pub mod agg;
pub mod predicate;
pub mod scalar;
pub mod schema;

use crate::catalog::view_name;
use crate::compiler::{
    agg::agg_to_sql,
    predicate::predicate_to_sql,
    scalar::{scalar_to_sql, sort_keys_sql},
};
use tv_core::{ColumnInfo, ViewExpr, ViewOp};

pub fn compile_tile(
    expr: &ViewExpr,
    base_schema: &[ColumnInfo],
    row: u64,
    col_offset: usize,
    rows: u64,
    cols: usize,
) -> String {
    let vname = view_name(&expr.source_id);
    let reordered = reorder_sort_last(&expr.ops);
    let virtual_schema =
        schema::infer_schema(base_schema, &reordered).unwrap_or_else(|_| base_schema.to_vec());

    let col_end = (col_offset + cols).min(virtual_schema.len());
    let select_cols: Vec<String> = virtual_schema[col_offset..col_end]
        .iter()
        .map(|c| format!("\"{}\"", c.name.replace('"', "\"\"")))
        .collect();
    let col_select = if select_cols.is_empty() {
        "*".to_string()
    } else {
        select_cols.join(", ")
    };

    let cte = build_cte_chain(&vname, &reordered, base_schema);
    let last = cte_last_alias(reordered.len());

    format!(
        "{} SELECT {col_select} FROM {last} LIMIT {rows} OFFSET {row}",
        cte
    )
}

pub fn compile_count(expr: &ViewExpr) -> String {
    let vname = view_name(&expr.source_id);
    let reordered = reorder_sort_last(&expr.ops);
    let cte = build_cte_chain(&vname, &reordered, &[]);
    let last = cte_last_alias(reordered.len());
    format!("{} SELECT COUNT(*) FROM {last}", cte)
}

pub fn compile_download(expr: &ViewExpr) -> String {
    let vname = view_name(&expr.source_id);
    let reordered = reorder_sort_last(&expr.ops);
    let cte = build_cte_chain(&vname, &reordered, &[]);
    let last = cte_last_alias(reordered.len());
    format!("{} SELECT * FROM {last}", cte)
}

fn build_cte_chain(vname: &str, ops: &[ViewOp], base_schema: &[ColumnInfo]) -> String {
    let mut current_schema = base_schema.to_vec();
    let mut steps: Vec<String> = vec![format!("_base AS (SELECT * FROM {vname})")];

    for (i, op) in ops.iter().enumerate() {
        let prev = if i == 0 {
            "_base".to_string()
        } else {
            format!("_s{}", i - 1)
        };
        let cte_sql = op_to_sql(op, &prev, &current_schema);
        steps.push(format!("_s{i} AS ({cte_sql})"));
        current_schema = schema::infer_schema_step(&current_schema, op)
            .unwrap_or_else(|_| current_schema.clone());
    }
    format!("WITH {} ", steps.join(", "))
}

fn cte_last_alias(op_count: usize) -> String {
    if op_count == 0 {
        "_base".to_string()
    } else {
        format!("_s{}", op_count - 1)
    }
}

fn op_to_sql(op: &ViewOp, from: &str, schema: &[ColumnInfo]) -> String {
    match op {
        ViewOp::Filter { predicate } => {
            format!("SELECT * FROM {from} WHERE {}", predicate_to_sql(predicate))
        }
        ViewOp::Select { columns } => {
            let cols: Vec<String> = columns
                .iter()
                .map(|c| format!("\"{}\"", c.replace('"', "\"\"")))
                .collect();
            format!("SELECT {} FROM {from}", cols.join(", "))
        }
        ViewOp::Drop { columns } => {
            let excluded: std::collections::HashSet<&String> = columns.iter().collect();
            let remaining: Vec<String> = schema
                .iter()
                .filter(|c| !excluded.contains(&c.name))
                .map(|c| format!("\"{}\"", c.name.replace('"', "\"\"")))
                .collect();
            if remaining.is_empty() {
                format!("SELECT * FROM {from}")
            } else {
                format!("SELECT {} FROM {from}", remaining.join(", "))
            }
        }
        ViewOp::Sort { keys } => {
            format!("SELECT * FROM {from} ORDER BY {}", sort_keys_sql(keys))
        }
        ViewOp::Derive { name, expr } => {
            let escaped = name.replace('"', "\"\"");
            format!(
                "SELECT *, {} AS \"{}\" FROM {from}",
                scalar_to_sql(expr),
                escaped
            )
        }
        ViewOp::Deduplicate { columns: None } => {
            format!("SELECT DISTINCT * FROM {from}")
        }
        ViewOp::Deduplicate {
            columns: Some(cols),
        } => {
            let partition: Vec<String> = cols
                .iter()
                .map(|c| format!("\"{}\"", c.replace('"', "\"\"")))
                .collect();
            let all_cols: Vec<String> = schema
                .iter()
                .map(|c| format!("\"{}\"", c.name.replace('"', "\"\"")))
                .collect();
            let select = if all_cols.is_empty() {
                "*".to_string()
            } else {
                all_cols.join(", ")
            };
            format!(
                "SELECT {select} FROM (SELECT *, ROW_NUMBER() OVER (PARTITION BY {}) as _rn FROM {from}) WHERE _rn = 1",
                partition.join(", ")
            )
        }
        ViewOp::Sample { n, .. } => {
            format!("SELECT * FROM {from} LIMIT {n}")
        }
        ViewOp::Rename { mappings } => {
            let renames: Vec<String> = schema
                .iter()
                .map(|col| {
                    let new_name = mappings
                        .iter()
                        .find(|(f, _)| f == &col.name)
                        .map(|(_, t)| t.as_str())
                        .unwrap_or(&col.name);
                    format!(
                        "\"{}\" AS \"{}\"",
                        col.name.replace('"', "\"\""),
                        new_name.replace('"', "\"\"")
                    )
                })
                .collect();
            format!("SELECT {} FROM {from}", renames.join(", "))
        }
        ViewOp::Limit { n } => format!("SELECT * FROM {from} LIMIT {n}"),
        ViewOp::GroupBy { keys, aggs } => {
            let key_cols: Vec<String> = keys
                .iter()
                .map(|k| format!("\"{}\"", k.replace('"', "\"\"")))
                .collect();
            let agg_exprs: Vec<String> = aggs.iter().map(agg_to_sql).collect();

            let mut select_parts = key_cols.clone();
            select_parts.extend(agg_exprs);

            let group_by = if key_cols.is_empty() {
                String::new()
            } else {
                format!(" GROUP BY {}", key_cols.join(", "))
            };

            format!("SELECT {} FROM {from}{group_by}", select_parts.join(", "))
        }

        ViewOp::TopK { .. } | ViewOp::Approximate { .. } => format!("SELECT * FROM {from}"),
    }
}

pub fn reorder_sort_last(ops: &[ViewOp]) -> Vec<ViewOp> {
    let mut non_sort: Vec<ViewOp> = ops
        .iter()
        .filter(|op| !matches!(op, ViewOp::Sort { .. }))
        .cloned()
        .collect();
    if let Some(sort) = ops
        .iter()
        .rev()
        .find(|op| matches!(op, ViewOp::Sort { .. }))
        .cloned()
    {
        non_sort.push(sort);
    }
    non_sort
}
