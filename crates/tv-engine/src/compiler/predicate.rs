use tv_core::{Literal, Predicate};

pub fn predicate_to_sql(pred: &Predicate) -> String {
    match pred {
        Predicate::Eq { column, value } => format!("\"{}\" = {}", esc(column), lit(value)),
        Predicate::Ne { column, value } => format!("\"{}\" != {}", esc(column), lit(value)),
        Predicate::Gt { column, value } => format!("\"{}\" > {}", esc(column), lit(value)),
        Predicate::Gte { column, value } => format!("\"{}\" >= {}", esc(column), lit(value)),
        Predicate::Lt { column, value } => format!("\"{}\" < {}", esc(column), lit(value)),
        Predicate::Lte { column, value } => format!("\"{}\" <= {}", esc(column), lit(value)),
        Predicate::Between { column, lo, hi } => {
            format!("\"{}\" BETWEEN {} AND {}", esc(column), lit(lo), lit(hi))
        }
        Predicate::In { column, values } => {
            let vals: Vec<String> = values.iter().map(lit).collect();
            format!("\"{}\" IN ({})", esc(column), vals.join(", "))
        }
        Predicate::NotIn { column, values } => {
            let vals: Vec<String> = values.iter().map(lit).collect();
            format!("\"{}\" NOT IN ({})", esc(column), vals.join(", "))
        }
        Predicate::Contains { column, value } => {
            format!("\"{}\" LIKE '%{}%'", esc(column), value.replace('\'', "''"))
        }
        Predicate::StartsWith { column, value } => {
            format!("\"{}\" LIKE '{}%'", esc(column), value.replace('\'', "''"))
        }
        Predicate::EndsWith { column, value } => {
            format!("\"{}\" LIKE '%{}'", esc(column), value.replace('\'', "''"))
        }
        Predicate::Regex { column, pattern } => {
            format!(
                "regexp_matches(\"{}\", '{}')",
                esc(column),
                pattern.replace('\'', "''")
            )
        }
        Predicate::IsNull { column } => format!("\"{}\" IS NULL", esc(column)),
        Predicate::IsNotNull { column } => format!("\"{}\" IS NOT NULL", esc(column)),
        Predicate::And { exprs } => {
            if exprs.is_empty() {
                return "TRUE".to_string();
            }
            let parts: Vec<String> = exprs.iter().map(predicate_to_sql).collect();
            format!("({})", parts.join(" AND "))
        }
        Predicate::Or { exprs } => {
            if exprs.is_empty() {
                return "FALSE".to_string();
            }
            let parts: Vec<String> = exprs.iter().map(predicate_to_sql).collect();
            format!("({})", parts.join(" OR "))
        }
        Predicate::Not { expr } => format!("NOT ({})", predicate_to_sql(expr)),
    }
}

pub fn esc(col: &str) -> String {
    col.replace('"', "\"\"")
}

pub fn lit(value: &Literal) -> String {
    match value {
        Literal::Null => "NULL".to_string(),
        Literal::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        Literal::Int(i) => i.to_string(),
        Literal::Float(f) => {
            if f.is_finite() {
                format!("{f}")
            } else if f.is_nan() {
                "'NaN'::DOUBLE".to_string()
            } else if *f > 0.0 {
                "'Infinity'::DOUBLE".to_string()
            } else {
                "'-Infinity'::DOUBLE".to_string()
            }
        }
        Literal::Text(s) => format!("'{}'", s.replace('\'', "''")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tv_core::Literal;

    #[test]
    fn eq_string() {
        let p = Predicate::Eq {
            column: "name".into(),
            value: Literal::Text("alice".into()),
        };
        assert_eq!(predicate_to_sql(&p), "\"name\" = 'alice'");
    }

    #[test]
    fn between_ints() {
        let p = Predicate::Between {
            column: "age".into(),
            lo: Literal::Int(18),
            hi: Literal::Int(65),
        };
        assert_eq!(predicate_to_sql(&p), "\"age\" BETWEEN 18 AND 65");
    }

    #[test]
    fn in_list() {
        let p = Predicate::In {
            column: "status".into(),
            values: vec![Literal::Text("a".into()), Literal::Text("b".into())],
        };
        assert_eq!(predicate_to_sql(&p), "\"status\" IN ('a', 'b')");
    }

    #[test]
    fn and_combines() {
        let p = Predicate::And {
            exprs: vec![
                Predicate::Gt {
                    column: "age".into(),
                    value: Literal::Int(30),
                },
                Predicate::IsNotNull {
                    column: "email".into(),
                },
            ],
        };
        assert_eq!(
            predicate_to_sql(&p),
            "(\"age\" > 30 AND \"email\" IS NOT NULL)"
        );
    }

    #[test]
    fn ne_float() {
        let p = Predicate::Ne {
            column: "score".into(),
            value: Literal::Float(3.14),
        };
        assert_eq!(predicate_to_sql(&p), "\"score\" != 3.14");
    }

    #[test]
    fn not_in() {
        let p = Predicate::NotIn {
            column: "status".into(),
            values: vec![Literal::Text("x".into()), Literal::Text("y".into())],
        };
        assert_eq!(predicate_to_sql(&p), "\"status\" NOT IN ('x', 'y')");
    }

    #[test]
    fn is_null_sql() {
        let p = Predicate::IsNull {
            column: "email".into(),
        };
        assert_eq!(predicate_to_sql(&p), "\"email\" IS NULL");
    }

    #[test]
    fn is_not_null_sql() {
        let p = Predicate::IsNotNull {
            column: "email".into(),
        };
        assert_eq!(predicate_to_sql(&p), "\"email\" IS NOT NULL");
    }

    #[test]
    fn contains_escapes_quotes() {
        let p = Predicate::Contains {
            column: "name".into(),
            value: "o'brien".into(),
        };
        let sql = predicate_to_sql(&p);
        assert!(sql.contains("brien"));
    }

    #[test]
    fn empty_and_returns_true() {
        let p = Predicate::And { exprs: vec![] };
        assert_eq!(predicate_to_sql(&p), "TRUE");
    }

    #[test]
    fn empty_or_returns_false() {
        let p = Predicate::Or { exprs: vec![] };
        assert_eq!(predicate_to_sql(&p), "FALSE");
    }

    #[test]
    fn not_wraps_correctly() {
        let p = Predicate::Not {
            expr: Box::new(Predicate::IsNull { column: "x".into() }),
        };
        assert_eq!(predicate_to_sql(&p), "NOT (\"x\" IS NULL)");
    }
}
