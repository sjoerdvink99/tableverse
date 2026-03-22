use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewExpr {
    pub source_id: String,
    pub ops: Vec<ViewOp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewOp {
    Filter {
        predicate: Predicate,
    },
    Select {
        columns: Vec<String>,
    },
    Drop {
        columns: Vec<String>,
    },
    Sort {
        keys: Vec<SortKey>,
    },
    Derive {
        name: String,
        expr: ScalarExpr,
    },
    Deduplicate {
        columns: Option<Vec<String>>,
    },
    Sample {
        n: u64,
        strategy: SampleStrategy,
        seed: Option<u64>,
    },
    GroupBy {
        keys: Vec<String>,
        aggs: Vec<AggExpr>,
    },
    Rename {
        mappings: Vec<(String, String)>,
    },
    Limit {
        n: u64,
    },
    TopK {
        n: u64,
        keys: Vec<SortKey>,
    },
    Approximate {
        sample_rows: u64,
        seed: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Predicate {
    Eq {
        column: String,
        value: Literal,
    },
    Ne {
        column: String,
        value: Literal,
    },
    Gt {
        column: String,
        value: Literal,
    },
    Gte {
        column: String,
        value: Literal,
    },
    Lt {
        column: String,
        value: Literal,
    },
    Lte {
        column: String,
        value: Literal,
    },
    Between {
        column: String,
        lo: Literal,
        hi: Literal,
    },
    In {
        column: String,
        values: Vec<Literal>,
    },
    NotIn {
        column: String,
        values: Vec<Literal>,
    },
    Contains {
        column: String,
        value: String,
    },
    StartsWith {
        column: String,
        value: String,
    },
    EndsWith {
        column: String,
        value: String,
    },
    Regex {
        column: String,
        pattern: String,
    },
    IsNull {
        column: String,
    },
    IsNotNull {
        column: String,
    },
    And {
        exprs: Vec<Predicate>,
    },
    Or {
        exprs: Vec<Predicate>,
    },
    Not {
        expr: Box<Predicate>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScalarExpr {
    Column {
        name: String,
    },
    Literal {
        value: Literal,
    },
    BinOp {
        op: BinOp,
        left: Box<ScalarExpr>,
        right: Box<ScalarExpr>,
    },
    Abs(Box<ScalarExpr>),
    Round {
        expr: Box<ScalarExpr>,
        decimals: i32,
    },
    Floor(Box<ScalarExpr>),
    Ceil(Box<ScalarExpr>),
    Upper(Box<ScalarExpr>),
    Lower(Box<ScalarExpr>),
    Trim(Box<ScalarExpr>),
    Length(Box<ScalarExpr>),
    Substr {
        expr: Box<ScalarExpr>,
        start: i64,
        len: Option<i64>,
    },
    Concat {
        parts: Vec<ScalarExpr>,
    },
    Year(Box<ScalarExpr>),
    Month(Box<ScalarExpr>),
    Day(Box<ScalarExpr>),
    Case {
        whens: Vec<(Predicate, ScalarExpr)>,
        else_expr: Option<Box<ScalarExpr>>,
    },
    Coalesce {
        exprs: Vec<ScalarExpr>,
    },
    Rank {
        order: Vec<SortKey>,
    },
    NTile {
        n: u64,
    },
    Cast {
        expr: Box<ScalarExpr>,
        to_type: DataType,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataType {
    Int32,
    Int64,
    Float32,
    Float64,
    Text,
    Boolean,
    Date,
    Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "fn", rename_all = "snake_case")]
pub enum AggExpr {
    Count {
        alias: String,
    },
    CountDistinct {
        column: String,
        alias: String,
    },
    Sum {
        column: String,
        alias: String,
    },
    Min {
        column: String,
        alias: String,
    },
    Max {
        column: String,
        alias: String,
    },
    Mean {
        column: String,
        alias: String,
    },
    Median {
        column: String,
        alias: String,
    },
    StdDev {
        column: String,
        alias: String,
    },
    Percentile {
        column: String,
        p: f64,
        alias: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortKey {
    pub column: String,
    pub descending: bool,
    pub nulls_last: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleStrategy {
    Bernoulli,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Literal {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

pub fn normalize_ops(ops: &[ViewOp]) -> Vec<ViewOp> {
    let mut selects: Vec<ViewOp> = Vec::new();
    let mut drops: Vec<ViewOp> = Vec::new();
    let mut filters: Vec<ViewOp> = Vec::new();
    let mut derives: Vec<ViewOp> = Vec::new();
    let mut deduplicates: Vec<ViewOp> = Vec::new();
    let mut group_bys: Vec<ViewOp> = Vec::new();
    let mut renames: Vec<ViewOp> = Vec::new();
    let mut sorts: Vec<ViewOp> = Vec::new();
    let mut limits: Vec<ViewOp> = Vec::new();
    let mut approximates: Vec<ViewOp> = Vec::new();

    for op in ops {
        match op {
            ViewOp::Select { .. } => selects.push(op.clone()),
            ViewOp::Drop { .. } => drops.push(op.clone()),
            ViewOp::Filter { .. } => filters.push(op.clone()),
            ViewOp::Derive { .. } => derives.push(op.clone()),
            ViewOp::Deduplicate { .. } => deduplicates.push(op.clone()),
            ViewOp::GroupBy { .. } => group_bys.push(op.clone()),
            ViewOp::Rename { .. } => renames.push(op.clone()),
            ViewOp::Sort { .. } => sorts.push(op.clone()),
            ViewOp::Limit { .. } => limits.push(op.clone()),
            ViewOp::Sample { .. } => deduplicates.push(op.clone()),
            ViewOp::TopK { .. } => sorts.push(op.clone()),
            ViewOp::Approximate { .. } => approximates.push(op.clone()),
        }
    }

    let mut result = Vec::new();
    result.extend(approximates);
    result.extend(selects);
    result.extend(drops);
    result.extend(filters);
    result.extend(derives);
    result.extend(deduplicates);
    result.extend(group_bys);
    result.extend(renames);
    result.extend(sorts);
    result.extend(limits);
    result
}

pub fn agg_alias(agg: &AggExpr) -> &str {
    match agg {
        AggExpr::Count { alias } => alias,
        AggExpr::CountDistinct { alias, .. } => alias,
        AggExpr::Sum { alias, .. } => alias,
        AggExpr::Min { alias, .. } => alias,
        AggExpr::Max { alias, .. } => alias,
        AggExpr::Mean { alias, .. } => alias,
        AggExpr::Median { alias, .. } => alias,
        AggExpr::StdDev { alias, .. } => alias,
        AggExpr::Percentile { alias, .. } => alias,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ops_sorts_last() {
        let ops = vec![
            ViewOp::Sort {
                keys: vec![SortKey {
                    column: "a".into(),
                    descending: false,
                    nulls_last: true,
                }],
            },
            ViewOp::Filter {
                predicate: Predicate::IsNotNull { column: "a".into() },
            },
        ];
        let normalized = normalize_ops(&ops);
        let last = normalized.last().unwrap();
        assert!(matches!(last, ViewOp::Sort { .. }));
    }

    #[test]
    fn normalize_ops_filters_before_derives() {
        let ops = vec![
            ViewOp::Derive {
                name: "d".into(),
                expr: ScalarExpr::Literal {
                    value: Literal::Int(1),
                },
            },
            ViewOp::Filter {
                predicate: Predicate::IsNull { column: "x".into() },
            },
        ];
        let normalized = normalize_ops(&ops);
        let filter_pos = normalized
            .iter()
            .position(|o| matches!(o, ViewOp::Filter { .. }))
            .unwrap();
        let derive_pos = normalized
            .iter()
            .position(|o| matches!(o, ViewOp::Derive { .. }))
            .unwrap();
        assert!(filter_pos < derive_pos);
    }

    #[test]
    fn normalize_ops_empty() {
        let normalized = normalize_ops(&[]);
        assert!(normalized.is_empty());
    }

    #[test]
    fn normalize_ops_single_op() {
        let ops = vec![ViewOp::Limit { n: 10 }];
        let normalized = normalize_ops(&ops);
        assert_eq!(normalized.len(), 1);
        assert!(matches!(normalized[0], ViewOp::Limit { n: 10 }));
    }

    #[test]
    fn normalize_ops_topk_grouped_with_sorts() {
        let ops = vec![
            ViewOp::TopK {
                n: 5,
                keys: vec![SortKey {
                    column: "x".into(),
                    descending: true,
                    nulls_last: true,
                }],
            },
            ViewOp::Filter {
                predicate: Predicate::IsNotNull { column: "x".into() },
            },
        ];
        let normalized = normalize_ops(&ops);
        let topk_pos = normalized
            .iter()
            .position(|o| matches!(o, ViewOp::TopK { .. }))
            .unwrap();
        let filter_pos = normalized
            .iter()
            .position(|o| matches!(o, ViewOp::Filter { .. }))
            .unwrap();
        assert!(filter_pos < topk_pos);
    }

    #[test]
    fn agg_alias_all_variants() {
        assert_eq!(
            agg_alias(&AggExpr::Count {
                alias: "cnt".into()
            }),
            "cnt"
        );
        assert_eq!(
            agg_alias(&AggExpr::Sum {
                column: "x".into(),
                alias: "s".into()
            }),
            "s"
        );
        assert_eq!(
            agg_alias(&AggExpr::Min {
                column: "x".into(),
                alias: "mn".into()
            }),
            "mn"
        );
        assert_eq!(
            agg_alias(&AggExpr::Max {
                column: "x".into(),
                alias: "mx".into()
            }),
            "mx"
        );
        assert_eq!(
            agg_alias(&AggExpr::Mean {
                column: "x".into(),
                alias: "avg".into()
            }),
            "avg"
        );
        assert_eq!(
            agg_alias(&AggExpr::Median {
                column: "x".into(),
                alias: "med".into()
            }),
            "med"
        );
        assert_eq!(
            agg_alias(&AggExpr::StdDev {
                column: "x".into(),
                alias: "sd".into()
            }),
            "sd"
        );
        assert_eq!(
            agg_alias(&AggExpr::Percentile {
                column: "x".into(),
                p: 95.0,
                alias: "p95".into()
            }),
            "p95"
        );
        assert_eq!(
            agg_alias(&AggExpr::CountDistinct {
                column: "x".into(),
                alias: "cd".into()
            }),
            "cd"
        );
    }

    #[test]
    fn literal_serde_roundtrip() {
        let lit_int = Literal::Int(42);
        let json = serde_json::json!(42);
        let back: Literal = serde_json::from_value(json).unwrap();
        assert!(matches!(back, Literal::Int(42)));

        let lit_text = Literal::Text("hello".into());
        let json2 = serde_json::json!("hello");
        let back2: Literal = serde_json::from_value(json2).unwrap();
        assert!(matches!(back2, Literal::Text(ref s) if s == "hello"));

        let _ = lit_int;
        let _ = lit_text;
    }

    #[test]
    fn viewop_serde_roundtrip() {
        let json = serde_json::json!({
            "type": "filter",
            "predicate": {
                "op": "eq",
                "column": "name",
                "value": "alice"
            }
        });
        let op: ViewOp = serde_json::from_value(json).unwrap();
        assert!(matches!(op, ViewOp::Filter { .. }));
    }
}
