use crate::compiler::agg::agg_to_sql;
use crate::compiler::predicate::{esc, predicate_to_sql};
use crate::compiler::reorder_sort_last;
use crate::compiler::scalar::{scalar_to_sql, sort_keys_sql};
use tv_core::{ViewExpr, ViewOp};

pub fn render_dbt(expr: &ViewExpr, source_name: &str) -> String {
    let ops = reorder_sort_last(&expr.ops);

    let mut steps: Vec<(String, String)> = Vec::new();
    let source_cte = format!(
        "SELECT * FROM {{{{ source('your_schema', '{}') }}}}",
        esc(source_name)
    );
    steps.push(("source".to_string(), source_cte));

    let mut current_alias = "source".to_string();

    for (i, op) in ops.iter().enumerate() {
        let step_name = format!("step_{i}");
        let sql = op_to_dbt_sql(op, &current_alias);
        steps.push((step_name.clone(), sql));
        current_alias = step_name;
    }

    steps.push((
        "final".to_string(),
        format!("SELECT * FROM {current_alias}"),
    ));

    let cte_blocks: Vec<String> = steps
        .iter()
        .map(|(name, body)| format!("{name} AS (\n    {}\n)", body.replace('\n', "\n    ")))
        .collect();

    format!(
        "{{{{ config(materialized='view') }}}}\n\nWITH {}\n\nSELECT * FROM final",
        cte_blocks.join(",\n\n")
    )
}

fn op_to_dbt_sql(op: &ViewOp, from: &str) -> String {
    match op {
        ViewOp::Filter { predicate } => {
            format!("SELECT * FROM {from} WHERE {}", predicate_to_sql(predicate))
        }
        ViewOp::Select { columns } => {
            let cols: Vec<String> = columns.iter().map(|c| format!("\"{}\"", esc(c))).collect();
            format!("SELECT {} FROM {from}", cols.join(", "))
        }
        ViewOp::Drop { columns } => {
            let excluded: Vec<String> = columns.iter().map(|c| format!("\"{}\"", esc(c))).collect();
            if excluded.is_empty() {
                format!("SELECT * FROM {from}")
            } else {
                format!("SELECT * EXCEPT ({}) FROM {from}", excluded.join(", "))
            }
        }
        ViewOp::Sort { keys } => {
            format!("SELECT * FROM {from} ORDER BY {}", sort_keys_sql(keys))
        }
        ViewOp::Derive { name, expr } => {
            format!(
                "SELECT *, {} AS \"{}\" FROM {from}",
                scalar_to_sql(expr),
                esc(name)
            )
        }
        ViewOp::Deduplicate { columns: None } => {
            format!("SELECT DISTINCT * FROM {from}")
        }
        ViewOp::Deduplicate {
            columns: Some(cols),
        } => {
            let partition: Vec<String> = cols.iter().map(|c| format!("\"{}\"", esc(c))).collect();
            format!(
                "SELECT * FROM (SELECT *, ROW_NUMBER() OVER (PARTITION BY {}) as _rn FROM {from}) WHERE _rn = 1",
                partition.join(", ")
            )
        }
        ViewOp::Sample { n, .. } => {
            format!("SELECT * FROM {from} LIMIT {n}")
        }
        ViewOp::GroupBy { keys, aggs } => {
            let key_cols: Vec<String> = keys.iter().map(|k| format!("\"{}\"", esc(k))).collect();
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
        ViewOp::Rename { mappings } => {
            let renames: Vec<String> = mappings
                .iter()
                .map(|(from_col, to_col)| format!("\"{}\" AS \"{}\"", esc(from_col), esc(to_col)))
                .collect();
            if renames.is_empty() {
                format!("SELECT * FROM {from}")
            } else {
                format!("SELECT * REPLACE ({}) FROM {from}", renames.join(", "))
            }
        }
        ViewOp::Limit { n } => format!("SELECT * FROM {from} LIMIT {n}"),
        ViewOp::TopK { .. } | ViewOp::Approximate { .. } => format!("SELECT * FROM {from}"),
    }
}
