use crate::compiler::reorder_sort_last;
use tv_core::{
    AggExpr, BinOp, DataType, Literal, Predicate, ScalarExpr, SourceFormat, ViewExpr, ViewOp,
};

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PythonDialect {
    DuckDb,
    Polars,
    Pandas,
}

pub fn render_python(
    expr: &ViewExpr,
    uri: &str,
    format: &SourceFormat,
    dialect: PythonDialect,
) -> String {
    match dialect {
        PythonDialect::DuckDb => render_duckdb(expr, uri, format),
        PythonDialect::Polars => render_polars(expr, uri, format),
        PythonDialect::Pandas => render_pandas(expr, uri, format),
    }
}

fn render_duckdb(expr: &ViewExpr, uri: &str, format: &SourceFormat) -> String {
    let sql = crate::export::sql::render_sql(expr, uri, format);
    let indented = sql.replace('\n', "\n    ");
    format!("import duckdb\n\ndf = duckdb.sql(\"\"\"\n    {indented}\n\"\"\").df()")
}

fn render_polars(expr: &ViewExpr, uri: &str, format: &SourceFormat) -> String {
    let scan = match format {
        SourceFormat::Parquet => format!("pl.scan_parquet(\"{}\")", esc_py(uri)),
        SourceFormat::Csv => format!("pl.scan_csv(\"{}\")", esc_py(uri)),
        SourceFormat::Arrow => format!("pl.scan_ipc(\"{}\")", esc_py(uri)),
        SourceFormat::Json => format!("pl.read_ndjson(\"{}\")", esc_py(uri)),
        SourceFormat::Delta | SourceFormat::Iceberg | SourceFormat::Database => {
            format!("pl.scan_parquet(\"{}\")", esc_py(uri))
        }
    };

    let mut chain = vec![format!("    {scan}")];
    let ops = reorder_sort_last(&expr.ops);

    for op in &ops {
        match op {
            ViewOp::Filter { predicate } => {
                chain.push(format!(
                    "    .filter(\n        {}\n    )",
                    pred_polars(predicate)
                ));
            }
            ViewOp::Select { columns } => {
                let cols: Vec<String> = columns
                    .iter()
                    .map(|c| format!("\"{}\"", esc_py(c)))
                    .collect();
                chain.push(format!("    .select([{}])", cols.join(", ")));
            }
            ViewOp::Drop { columns } => {
                let cols: Vec<String> = columns
                    .iter()
                    .map(|c| format!("\"{}\"", esc_py(c)))
                    .collect();
                chain.push(format!("    .drop([{}])", cols.join(", ")));
            }
            ViewOp::Sort { keys } => {
                if keys.len() == 1 {
                    let desc = if keys[0].descending { "True" } else { "False" };
                    chain.push(format!(
                        "    .sort(\"{}\", descending={})",
                        esc_py(&keys[0].column),
                        desc
                    ));
                } else {
                    let cols: Vec<String> = keys
                        .iter()
                        .map(|k| format!("\"{}\"", esc_py(&k.column)))
                        .collect();
                    let descs: Vec<&str> = keys
                        .iter()
                        .map(|k| if k.descending { "True" } else { "False" })
                        .collect();
                    chain.push(format!(
                        "    .sort([{}], descending=[{}])",
                        cols.join(", "),
                        descs.join(", ")
                    ));
                }
            }
            ViewOp::Derive { name, expr } => {
                chain.push(format!(
                    "    .with_columns(\n        {}.alias(\"{}\")\n    )",
                    scalar_polars(expr),
                    esc_py(name)
                ));
            }
            ViewOp::Deduplicate { columns: None } => {
                chain.push("    .unique()".to_string());
            }
            ViewOp::Deduplicate {
                columns: Some(cols),
            } => {
                let col_list: Vec<String> =
                    cols.iter().map(|c| format!("\"{}\"", esc_py(c))).collect();
                chain.push(format!("    .unique(subset=[{}])", col_list.join(", ")));
            }
            ViewOp::Sample { n, .. } => {
                chain.push(format!("    .sample(n={})", n));
            }
            ViewOp::GroupBy { keys, aggs } => {
                let key_cols: Vec<String> =
                    keys.iter().map(|k| format!("\"{}\"", esc_py(k))).collect();
                let agg_exprs: Vec<String> = aggs.iter().map(agg_polars).collect();
                chain.push(format!(
                    "    .group_by([{}])\n    .agg([\n        {}\n    ])",
                    key_cols.join(", "),
                    agg_exprs.join(",\n        ")
                ));
            }
            ViewOp::Rename { mappings } => {
                let pairs: Vec<String> = mappings
                    .iter()
                    .map(|(from, to)| format!("\"{}\": \"{}\"", esc_py(from), esc_py(to)))
                    .collect();
                chain.push(format!("    .rename({{{}}})", pairs.join(", ")));
            }
            ViewOp::Limit { n } => {
                chain.push(format!("    .limit({})", n));
            }
            ViewOp::TopK { .. } | ViewOp::Approximate { .. } => {}
        }
    }

    chain.push("    .collect()".to_string());
    format!("import polars as pl\n\ndf = (\n{}\n)", chain.join("\n"))
}

fn render_pandas(expr: &ViewExpr, uri: &str, format: &SourceFormat) -> String {
    let read = match format {
        SourceFormat::Parquet => format!("pd.read_parquet(\"{}\")", esc_py(uri)),
        SourceFormat::Csv => format!("pd.read_csv(\"{}\")", esc_py(uri)),
        SourceFormat::Arrow => format!("pd.read_feather(\"{}\")", esc_py(uri)),
        SourceFormat::Json => format!("pd.read_json(\"{}\")", esc_py(uri)),
        SourceFormat::Delta | SourceFormat::Iceberg | SourceFormat::Database => {
            format!("pd.read_parquet(\"{}\")", esc_py(uri))
        }
    };

    let mut lines = vec![
        "import pandas as pd".to_string(),
        String::new(),
        format!("df = {read}"),
    ];
    let ops = reorder_sort_last(&expr.ops);

    for op in &ops {
        match op {
            ViewOp::Filter { predicate } => {
                lines.push(format!("df = df[{}]", pred_pandas(predicate)));
            }
            ViewOp::Select { columns } => {
                let cols: Vec<String> = columns
                    .iter()
                    .map(|c| format!("\"{}\"", esc_py(c)))
                    .collect();
                lines.push(format!("df = df[[{}]]", cols.join(", ")));
            }
            ViewOp::Drop { columns } => {
                let cols: Vec<String> = columns
                    .iter()
                    .map(|c| format!("\"{}\"", esc_py(c)))
                    .collect();
                lines.push(format!("df = df.drop(columns=[{}])", cols.join(", ")));
            }
            ViewOp::Sort { keys } => {
                let cols: Vec<String> = keys
                    .iter()
                    .map(|k| format!("\"{}\"", esc_py(&k.column)))
                    .collect();
                let ascs: Vec<&str> = keys
                    .iter()
                    .map(|k| if k.descending { "False" } else { "True" })
                    .collect();
                lines.push(format!(
                    "df = df.sort_values(by=[{}], ascending=[{}])",
                    cols.join(", "),
                    ascs.join(", ")
                ));
            }
            ViewOp::Derive { name, expr } => {
                lines.push(format!(
                    "df[\"{}\"] = {}",
                    esc_py(name),
                    scalar_pandas(expr)
                ));
            }
            ViewOp::Deduplicate { columns: None } => {
                lines.push("df = df.drop_duplicates()".to_string());
            }
            ViewOp::Deduplicate {
                columns: Some(cols),
            } => {
                let col_list: Vec<String> =
                    cols.iter().map(|c| format!("\"{}\"", esc_py(c))).collect();
                lines.push(format!(
                    "df = df.drop_duplicates(subset=[{}])",
                    col_list.join(", ")
                ));
            }
            ViewOp::Sample { n, .. } => {
                lines.push(format!("df = df.sample(n={})", n));
            }
            ViewOp::GroupBy { keys, aggs } => {
                let key_cols: Vec<String> =
                    keys.iter().map(|k| format!("\"{}\"", esc_py(k))).collect();
                let agg_dict = pandas_agg_dict(aggs);
                lines.push(format!(
                    "df = df.groupby([{}]).agg({agg_dict}).reset_index()",
                    key_cols.join(", ")
                ));
            }
            ViewOp::Rename { mappings } => {
                let pairs: Vec<String> = mappings
                    .iter()
                    .map(|(from, to)| format!("\"{}\": \"{}\"", esc_py(from), esc_py(to)))
                    .collect();
                lines.push(format!("df = df.rename(columns={{{}}})", pairs.join(", ")));
            }
            ViewOp::Limit { n } => {
                lines.push(format!("df = df.head({})", n));
            }
            ViewOp::TopK { .. } | ViewOp::Approximate { .. } => {}
        }
    }

    let uses_numpy = lines.iter().any(|l| l.contains("np."));
    if uses_numpy && !lines.contains(&"import numpy as np".to_string()) {
        lines.insert(1, "import numpy as np".to_string());
    }

    lines.join("\n")
}

fn pred_polars(pred: &Predicate) -> String {
    match pred {
        Predicate::Eq { column, value } => {
            format!("(pl.col(\"{}\") == {})", esc_py(column), lit_py(value))
        }
        Predicate::Ne { column, value } => {
            format!("(pl.col(\"{}\") != {})", esc_py(column), lit_py(value))
        }
        Predicate::Gt { column, value } => {
            format!("(pl.col(\"{}\") > {})", esc_py(column), lit_py(value))
        }
        Predicate::Gte { column, value } => {
            format!("(pl.col(\"{}\") >= {})", esc_py(column), lit_py(value))
        }
        Predicate::Lt { column, value } => {
            format!("(pl.col(\"{}\") < {})", esc_py(column), lit_py(value))
        }
        Predicate::Lte { column, value } => {
            format!("(pl.col(\"{}\") <= {})", esc_py(column), lit_py(value))
        }
        Predicate::Between { column, lo, hi } => {
            format!(
                "pl.col(\"{}\").is_between({}, {})",
                esc_py(column),
                lit_py(lo),
                lit_py(hi)
            )
        }
        Predicate::In { column, values } => {
            let vals: Vec<String> = values.iter().map(lit_py).collect();
            format!(
                "pl.col(\"{}\").is_in([{}])",
                esc_py(column),
                vals.join(", ")
            )
        }
        Predicate::NotIn { column, values } => {
            let vals: Vec<String> = values.iter().map(lit_py).collect();
            format!(
                "~pl.col(\"{}\").is_in([{}])",
                esc_py(column),
                vals.join(", ")
            )
        }
        Predicate::Contains { column, value } => {
            format!(
                "pl.col(\"{}\").str.contains(\"{}\")",
                esc_py(column),
                esc_py(value)
            )
        }
        Predicate::StartsWith { column, value } => {
            format!(
                "pl.col(\"{}\").str.starts_with(\"{}\")",
                esc_py(column),
                esc_py(value)
            )
        }
        Predicate::EndsWith { column, value } => {
            format!(
                "pl.col(\"{}\").str.ends_with(\"{}\")",
                esc_py(column),
                esc_py(value)
            )
        }
        Predicate::Regex { column, pattern } => {
            format!(
                "pl.col(\"{}\").str.contains(r\"{}\", literal=False)",
                esc_py(column),
                esc_py(pattern)
            )
        }
        Predicate::IsNull { column } => format!("pl.col(\"{}\").is_null()", esc_py(column)),
        Predicate::IsNotNull { column } => format!("pl.col(\"{}\").is_not_null()", esc_py(column)),
        Predicate::And { exprs } => {
            if exprs.is_empty() {
                return "True".to_string();
            }
            exprs
                .iter()
                .map(pred_polars)
                .collect::<Vec<_>>()
                .join(" & ")
        }
        Predicate::Or { exprs } => {
            if exprs.is_empty() {
                return "False".to_string();
            }
            exprs
                .iter()
                .map(pred_polars)
                .collect::<Vec<_>>()
                .join(" | ")
        }
        Predicate::Not { expr } => format!("~({})", pred_polars(expr)),
    }
}

fn pred_pandas(pred: &Predicate) -> String {
    match pred {
        Predicate::Eq { column, value } => {
            format!("(df[\"{}\"] == {})", esc_py(column), lit_py(value))
        }
        Predicate::Ne { column, value } => {
            format!("(df[\"{}\"] != {})", esc_py(column), lit_py(value))
        }
        Predicate::Gt { column, value } => {
            format!("(df[\"{}\"] > {})", esc_py(column), lit_py(value))
        }
        Predicate::Gte { column, value } => {
            format!("(df[\"{}\"] >= {})", esc_py(column), lit_py(value))
        }
        Predicate::Lt { column, value } => {
            format!("(df[\"{}\"] < {})", esc_py(column), lit_py(value))
        }
        Predicate::Lte { column, value } => {
            format!("(df[\"{}\"] <= {})", esc_py(column), lit_py(value))
        }
        Predicate::Between { column, lo, hi } => {
            format!(
                "(df[\"{}\"].between({}, {}))",
                esc_py(column),
                lit_py(lo),
                lit_py(hi)
            )
        }
        Predicate::In { column, values } => {
            let vals: Vec<String> = values.iter().map(lit_py).collect();
            format!("df[\"{}\"].isin([{}])", esc_py(column), vals.join(", "))
        }
        Predicate::NotIn { column, values } => {
            let vals: Vec<String> = values.iter().map(lit_py).collect();
            format!("~df[\"{}\"].isin([{}])", esc_py(column), vals.join(", "))
        }
        Predicate::Contains { column, value } => {
            format!(
                "df[\"{}\"].str.contains(\"{}\")",
                esc_py(column),
                esc_py(value)
            )
        }
        Predicate::StartsWith { column, value } => {
            format!(
                "df[\"{}\"].str.startswith(\"{}\")",
                esc_py(column),
                esc_py(value)
            )
        }
        Predicate::EndsWith { column, value } => {
            format!(
                "df[\"{}\"].str.endswith(\"{}\")",
                esc_py(column),
                esc_py(value)
            )
        }
        Predicate::Regex { column, pattern } => {
            format!(
                "df[\"{}\"].str.match(r\"{}\")",
                esc_py(column),
                esc_py(pattern)
            )
        }
        Predicate::IsNull { column } => format!("df[\"{}\"].isna()", esc_py(column)),
        Predicate::IsNotNull { column } => format!("df[\"{}\"].notna()", esc_py(column)),
        Predicate::And { exprs } => {
            if exprs.is_empty() {
                return "True".to_string();
            }
            exprs
                .iter()
                .map(pred_pandas)
                .collect::<Vec<_>>()
                .join(" & ")
        }
        Predicate::Or { exprs } => {
            if exprs.is_empty() {
                return "False".to_string();
            }
            exprs
                .iter()
                .map(pred_pandas)
                .collect::<Vec<_>>()
                .join(" | ")
        }
        Predicate::Not { expr } => format!("~({})", pred_pandas(expr)),
    }
}

fn scalar_polars(expr: &ScalarExpr) -> String {
    match expr {
        ScalarExpr::Column { name } => format!("pl.col(\"{}\")", esc_py(name)),
        ScalarExpr::Literal { value } => lit_py(value),
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
                scalar_polars(left),
                scalar_polars(right)
            )
        }
        ScalarExpr::Abs(inner) => format!("{}.abs()", scalar_polars(inner)),
        ScalarExpr::Round { expr, decimals } => {
            format!("{}.round({})", scalar_polars(expr), decimals)
        }
        ScalarExpr::Floor(inner) => format!("{}.floor()", scalar_polars(inner)),
        ScalarExpr::Ceil(inner) => format!("{}.ceil()", scalar_polars(inner)),
        ScalarExpr::Upper(inner) => format!("{}.str.to_uppercase()", scalar_polars(inner)),
        ScalarExpr::Lower(inner) => format!("{}.str.to_lowercase()", scalar_polars(inner)),
        ScalarExpr::Trim(inner) => format!("{}.str.strip_chars()", scalar_polars(inner)),
        ScalarExpr::Length(inner) => format!("{}.str.len_chars()", scalar_polars(inner)),
        ScalarExpr::Year(inner) => format!("{}.dt.year()", scalar_polars(inner)),
        ScalarExpr::Month(inner) => format!("{}.dt.month()", scalar_polars(inner)),
        ScalarExpr::Day(inner) => format!("{}.dt.day()", scalar_polars(inner)),
        ScalarExpr::Cast { expr, to_type } => {
            let dtype = match to_type {
                DataType::Int32 => "pl.Int32",
                DataType::Int64 => "pl.Int64",
                DataType::Float32 => "pl.Float32",
                DataType::Float64 => "pl.Float64",
                DataType::Text => "pl.Utf8",
                DataType::Boolean => "pl.Boolean",
                DataType::Date => "pl.Date",
                DataType::Timestamp => "pl.Datetime",
            };
            format!("{}.cast({})", scalar_polars(expr), dtype)
        }
        ScalarExpr::Coalesce { exprs } => {
            let parts: Vec<String> = exprs.iter().map(scalar_polars).collect();
            format!("pl.coalesce([{}])", parts.join(", "))
        }
        ScalarExpr::Substr { expr, start, len } => {
            let offset = *start;
            let length_part = len.map(|l| format!(", length={l}")).unwrap_or_default();
            format!("{}.str.slice({offset}{length_part})", scalar_polars(expr))
        }
        ScalarExpr::Concat { parts } => {
            let exprs: Vec<String> = parts
                .iter()
                .map(|p| format!("{}.cast(pl.Utf8)", scalar_polars(p)))
                .collect();
            format!("pl.concat_str([{}])", exprs.join(", "))
        }
        ScalarExpr::Case { whens, else_expr } => {
            if whens.is_empty() {
                return else_expr
                    .as_ref()
                    .map(|e| scalar_polars(e))
                    .unwrap_or("pl.lit(None)".to_string());
            }
            let mut result = format!(
                "pl.when({}).then({})",
                pred_polars(&whens[0].0),
                scalar_polars(&whens[0].1)
            );
            for (pred, then) in &whens[1..] {
                result = format!(
                    "{result}.when({}).then({})",
                    pred_polars(pred),
                    scalar_polars(then)
                );
            }
            let otherwise = else_expr
                .as_ref()
                .map(|e| scalar_polars(e))
                .unwrap_or("pl.lit(None)".to_string());
            format!("{result}.otherwise({otherwise})")
        }
        ScalarExpr::Rank { order } => {
            if order.is_empty() {
                "pl.lit(0).rank()".to_string()
            } else {
                let sort_col = &order[0].column;
                let desc = order[0].descending;
                format!(
                    "pl.col(\"{}\").rank(descending={})",
                    esc_py(sort_col),
                    if desc { "True" } else { "False" }
                )
            }
        }
        ScalarExpr::NTile { n } => {
            format!("pl.int_range(pl.len()).floordiv(pl.len() // {n} + 1).add(1).clip(1, {n})")
        }
    }
}

fn scalar_pandas(expr: &ScalarExpr) -> String {
    match expr {
        ScalarExpr::Column { name } => format!("df[\"{}\"]", esc_py(name)),
        ScalarExpr::Literal { value } => lit_py(value),
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
                scalar_pandas(left),
                scalar_pandas(right)
            )
        }
        ScalarExpr::Abs(inner) => format!("{}.abs()", scalar_pandas(inner)),
        ScalarExpr::Round { expr, decimals } => {
            format!("{}.round({})", scalar_pandas(expr), decimals)
        }
        ScalarExpr::Upper(inner) => format!("{}.str.upper()", scalar_pandas(inner)),
        ScalarExpr::Lower(inner) => format!("{}.str.lower()", scalar_pandas(inner)),
        ScalarExpr::Trim(inner) => format!("{}.str.strip()", scalar_pandas(inner)),
        ScalarExpr::Length(inner) => format!("{}.str.len()", scalar_pandas(inner)),
        ScalarExpr::Year(inner) => format!("{}.dt.year", scalar_pandas(inner)),
        ScalarExpr::Month(inner) => format!("{}.dt.month", scalar_pandas(inner)),
        ScalarExpr::Day(inner) => format!("{}.dt.day", scalar_pandas(inner)),
        ScalarExpr::Cast { expr, to_type } => {
            let dtype = match to_type {
                DataType::Int32 => "\"int32\"",
                DataType::Int64 => "\"int64\"",
                DataType::Float32 => "\"float32\"",
                DataType::Float64 => "\"float64\"",
                DataType::Text => "str",
                DataType::Boolean => "bool",
                DataType::Date | DataType::Timestamp => "\"datetime64[ns]\"",
            };
            format!("{}.astype({})", scalar_pandas(expr), dtype)
        }
        ScalarExpr::Floor(inner) => format!("np.floor({})", scalar_pandas(inner)),
        ScalarExpr::Ceil(inner) => format!("np.ceil({})", scalar_pandas(inner)),
        ScalarExpr::Substr { expr, start, len } => {
            let end = len.map(|l| format!("{}", start + l)).unwrap_or_default();
            format!("{}.str[{}:{}]", scalar_pandas(expr), start, end)
        }
        ScalarExpr::Concat { parts } => parts
            .iter()
            .map(scalar_pandas)
            .collect::<Vec<_>>()
            .join(" + "),
        ScalarExpr::Coalesce { exprs } => {
            if exprs.is_empty() {
                return "None".to_string();
            }
            let first = scalar_pandas(&exprs[0]);
            exprs[1..].iter().fold(first, |acc, e| {
                format!("{acc}.fillna({})", scalar_pandas(e))
            })
        }
        ScalarExpr::Case { whens, else_expr } => {
            let conditions: Vec<String> = whens.iter().map(|(p, _)| pred_pandas(p)).collect();
            let choices: Vec<String> = whens.iter().map(|(_, e)| scalar_pandas(e)).collect();
            let default = else_expr
                .as_ref()
                .map(|e| scalar_pandas(e))
                .unwrap_or("np.nan".to_string());
            format!(
                "np.select([{}], [{}], default={})",
                conditions.join(", "),
                choices.join(", "),
                default
            )
        }
        _ => "None".to_string(),
    }
}

fn agg_polars(agg: &AggExpr) -> String {
    match agg {
        AggExpr::Count { alias } => format!("pl.len().alias(\"{}\")", esc_py(alias)),
        AggExpr::CountDistinct { column, alias } => {
            format!(
                "pl.col(\"{}\").n_unique().alias(\"{}\")",
                esc_py(column),
                esc_py(alias)
            )
        }
        AggExpr::Sum { column, alias } => format!(
            "pl.col(\"{}\").sum().alias(\"{}\")",
            esc_py(column),
            esc_py(alias)
        ),
        AggExpr::Min { column, alias } => format!(
            "pl.col(\"{}\").min().alias(\"{}\")",
            esc_py(column),
            esc_py(alias)
        ),
        AggExpr::Max { column, alias } => format!(
            "pl.col(\"{}\").max().alias(\"{}\")",
            esc_py(column),
            esc_py(alias)
        ),
        AggExpr::Mean { column, alias } => format!(
            "pl.col(\"{}\").mean().alias(\"{}\")",
            esc_py(column),
            esc_py(alias)
        ),
        AggExpr::Median { column, alias } => {
            format!(
                "pl.col(\"{}\").median().alias(\"{}\")",
                esc_py(column),
                esc_py(alias)
            )
        }
        AggExpr::StdDev { column, alias } => format!(
            "pl.col(\"{}\").std().alias(\"{}\")",
            esc_py(column),
            esc_py(alias)
        ),
        AggExpr::Percentile { column, p, alias } => {
            format!(
                "pl.col(\"{}\").quantile({p}).alias(\"{}\")",
                esc_py(column),
                esc_py(alias)
            )
        }
    }
}

fn pandas_agg_dict(aggs: &[AggExpr]) -> String {
    let entries: Vec<String> = aggs
        .iter()
        .filter_map(|agg| match agg {
            AggExpr::Count { alias } => Some(format!("\"{}\": \"count\"", esc_py(alias))),
            AggExpr::CountDistinct { column, alias } => Some(format!(
                "\"{}\": (\"{}\", \"nunique\")",
                esc_py(alias),
                esc_py(column)
            )),
            AggExpr::Sum { column, alias } => Some(format!(
                "\"{}\": (\"{}\", \"sum\")",
                esc_py(alias),
                esc_py(column)
            )),
            AggExpr::Min { column, alias } => Some(format!(
                "\"{}\": (\"{}\", \"min\")",
                esc_py(alias),
                esc_py(column)
            )),
            AggExpr::Max { column, alias } => Some(format!(
                "\"{}\": (\"{}\", \"max\")",
                esc_py(alias),
                esc_py(column)
            )),
            AggExpr::Mean { column, alias } => Some(format!(
                "\"{}\": (\"{}\", \"mean\")",
                esc_py(alias),
                esc_py(column)
            )),
            AggExpr::Median { column, alias } => Some(format!(
                "\"{}\": (\"{}\", \"median\")",
                esc_py(alias),
                esc_py(column)
            )),
            AggExpr::StdDev { column, alias } => Some(format!(
                "\"{}\": (\"{}\", \"std\")",
                esc_py(alias),
                esc_py(column)
            )),
            _ => None,
        })
        .collect();
    format!("{{{}}}", entries.join(", "))
}

fn esc_py(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn lit_py(value: &Literal) -> String {
    match value {
        Literal::Null => "None".to_string(),
        Literal::Bool(b) => {
            if *b {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Literal::Int(i) => i.to_string(),
        Literal::Float(f) => format!("{f}"),
        Literal::Text(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
    }
}
