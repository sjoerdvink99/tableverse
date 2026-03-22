use crate::compiler::predicate::esc;
use tv_core::AggExpr;

pub fn agg_to_sql(agg: &AggExpr) -> String {
    match agg {
        AggExpr::Count { alias } => format!("COUNT(*) AS \"{}\"", esc(alias)),
        AggExpr::CountDistinct { column, alias } => {
            format!("COUNT(DISTINCT \"{}\") AS \"{}\"", esc(column), esc(alias))
        }
        AggExpr::Sum { column, alias } => format!("SUM(\"{}\") AS \"{}\"", esc(column), esc(alias)),
        AggExpr::Min { column, alias } => format!("MIN(\"{}\") AS \"{}\"", esc(column), esc(alias)),
        AggExpr::Max { column, alias } => format!("MAX(\"{}\") AS \"{}\"", esc(column), esc(alias)),
        AggExpr::Mean { column, alias } => {
            format!("AVG(\"{}\") AS \"{}\"", esc(column), esc(alias))
        }
        AggExpr::Median { column, alias } => {
            format!("MEDIAN(\"{}\") AS \"{}\"", esc(column), esc(alias))
        }
        AggExpr::StdDev { column, alias } => {
            format!("STDDEV(\"{}\") AS \"{}\"", esc(column), esc(alias))
        }
        AggExpr::Percentile { column, p, alias } => format!(
            "PERCENTILE_CONT({p}) WITHIN GROUP (ORDER BY \"{}\") AS \"{}\"",
            esc(column),
            esc(alias)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tv_core::AggExpr;

    #[test]
    fn agg_count_to_sql() {
        let agg = AggExpr::Count {
            alias: "cnt".into(),
        };
        let sql = agg_to_sql(&agg);
        assert!(sql.to_uppercase().contains("COUNT(*)"));
        assert!(sql.contains("cnt"));
    }

    #[test]
    fn agg_sum_to_sql() {
        let agg = AggExpr::Sum {
            column: "salary".into(),
            alias: "total".into(),
        };
        let sql = agg_to_sql(&agg);
        assert!(sql.to_uppercase().contains("SUM"));
        assert!(sql.contains("salary"));
        assert!(sql.contains("total"));
    }

    #[test]
    fn agg_percentile_to_sql() {
        let agg = AggExpr::Percentile {
            column: "score".into(),
            p: 0.95,
            alias: "p95".into(),
        };
        let sql = agg_to_sql(&agg);
        assert!(sql.to_uppercase().contains("PERCENTILE"));
        assert!(sql.contains("0.95"));
        assert!(sql.contains("score"));
    }

    #[test]
    fn agg_count_distinct_to_sql() {
        let agg = AggExpr::CountDistinct {
            column: "user_id".into(),
            alias: "uniq".into(),
        };
        let sql = agg_to_sql(&agg);
        assert!(sql.to_uppercase().contains("COUNT(DISTINCT"));
        assert!(sql.contains("user_id"));
    }

    #[test]
    fn agg_min_max_to_sql() {
        let min_agg = AggExpr::Min {
            column: "age".into(),
            alias: "min_age".into(),
        };
        let max_agg = AggExpr::Max {
            column: "age".into(),
            alias: "max_age".into(),
        };
        assert!(agg_to_sql(&min_agg).to_uppercase().contains("MIN"));
        assert!(agg_to_sql(&max_agg).to_uppercase().contains("MAX"));
    }
}
