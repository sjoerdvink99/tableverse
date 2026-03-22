use crate::compiler::predicate::{esc, lit, predicate_to_sql};
use tv_core::{BinOp, DataType, ScalarExpr};

pub fn scalar_to_sql(expr: &ScalarExpr) -> String {
    match expr {
        ScalarExpr::Column { name } => format!("\"{}\"", esc(name)),
        ScalarExpr::Literal { value } => lit(value),
        ScalarExpr::BinOp { op, left, right } => {
            let op_str = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Mod => "%",
            };
            format!(
                "({} {op_str} {})",
                scalar_to_sql(left),
                scalar_to_sql(right)
            )
        }
        ScalarExpr::Abs(inner) => format!("abs({})", scalar_to_sql(inner)),
        ScalarExpr::Round { expr, decimals } => {
            format!("round({}, {decimals})", scalar_to_sql(expr))
        }
        ScalarExpr::Floor(inner) => format!("floor({})", scalar_to_sql(inner)),
        ScalarExpr::Ceil(inner) => format!("ceil({})", scalar_to_sql(inner)),
        ScalarExpr::Upper(inner) => format!("upper({})", scalar_to_sql(inner)),
        ScalarExpr::Lower(inner) => format!("lower({})", scalar_to_sql(inner)),
        ScalarExpr::Trim(inner) => format!("trim({})", scalar_to_sql(inner)),
        ScalarExpr::Length(inner) => format!("length({})", scalar_to_sql(inner)),
        ScalarExpr::Substr { expr, start, len } => match len {
            Some(l) => format!("substr({}, {start}, {l})", scalar_to_sql(expr)),
            None => format!("substr({}, {start})", scalar_to_sql(expr)),
        },
        ScalarExpr::Concat { parts } => {
            let sql_parts: Vec<String> = parts.iter().map(scalar_to_sql).collect();
            format!("concat({})", sql_parts.join(", "))
        }
        ScalarExpr::Year(inner) => format!("year({})", scalar_to_sql(inner)),
        ScalarExpr::Month(inner) => format!("month({})", scalar_to_sql(inner)),
        ScalarExpr::Day(inner) => format!("day({})", scalar_to_sql(inner)),
        ScalarExpr::Case { whens, else_expr } => {
            let mut sql = "CASE".to_string();
            for (pred, then) in whens {
                sql.push_str(&format!(
                    " WHEN {} THEN {}",
                    predicate_to_sql(pred),
                    scalar_to_sql(then)
                ));
            }
            if let Some(e) = else_expr {
                sql.push_str(&format!(" ELSE {}", scalar_to_sql(e)));
            }
            sql.push_str(" END");
            sql
        }
        ScalarExpr::Coalesce { exprs } => {
            let parts: Vec<String> = exprs.iter().map(scalar_to_sql).collect();
            format!("coalesce({})", parts.join(", "))
        }
        ScalarExpr::Rank { order } => {
            let order_sql = sort_keys_sql(order);
            format!("rank() OVER (ORDER BY {order_sql})")
        }
        ScalarExpr::NTile { n } => format!("ntile({n}) OVER ()"),
        ScalarExpr::Cast { expr, to_type } => {
            let type_str = match to_type {
                DataType::Int32 => "INTEGER",
                DataType::Int64 => "BIGINT",
                DataType::Float32 => "FLOAT",
                DataType::Float64 => "DOUBLE",
                DataType::Text => "VARCHAR",
                DataType::Boolean => "BOOLEAN",
                DataType::Date => "DATE",
                DataType::Timestamp => "TIMESTAMP",
            };
            format!("({})::{}", scalar_to_sql(expr), type_str)
        }
    }
}

pub fn sort_keys_sql(keys: &[tv_core::SortKey]) -> String {
    keys.iter()
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
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tv_core::{BinOp, DataType, Literal, ScalarExpr};

    #[test]
    fn scalar_column_to_sql() {
        let e = ScalarExpr::Column { name: "age".into() };
        assert_eq!(scalar_to_sql(&e), "\"age\"");
    }

    #[test]
    fn scalar_literal_text_to_sql() {
        let e = ScalarExpr::Literal {
            value: Literal::Text("hello".into()),
        };
        assert_eq!(scalar_to_sql(&e), "'hello'");
    }

    #[test]
    fn scalar_literal_int_to_sql() {
        let e = ScalarExpr::Literal {
            value: Literal::Int(42),
        };
        assert_eq!(scalar_to_sql(&e), "42");
    }

    #[test]
    fn scalar_bin_op_add_to_sql() {
        let e = ScalarExpr::BinOp {
            op: BinOp::Add,
            left: Box::new(ScalarExpr::Column { name: "x".into() }),
            right: Box::new(ScalarExpr::Literal {
                value: Literal::Int(1),
            }),
        };
        let sql = scalar_to_sql(&e);
        assert!(sql.contains("+"));
        assert!(sql.contains("\"x\""));
    }

    #[test]
    fn scalar_round_to_sql() {
        let e = ScalarExpr::Round {
            expr: Box::new(ScalarExpr::Column {
                name: "score".into(),
            }),
            decimals: 2,
        };
        let sql = scalar_to_sql(&e);
        assert!(sql.to_uppercase().contains("ROUND") || sql.contains("round"));
    }

    #[test]
    fn scalar_cast_to_sql() {
        let e = ScalarExpr::Cast {
            expr: Box::new(ScalarExpr::Column { name: "id".into() }),
            to_type: DataType::Float64,
        };
        let sql = scalar_to_sql(&e);
        assert!(sql.to_uppercase().contains("CAST") || sql.contains("cast") || sql.contains("::"));
    }
}
