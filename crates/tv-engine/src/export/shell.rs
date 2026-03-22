use tv_core::{SourceFormat, ViewExpr};

pub fn render_shell(expr: &ViewExpr, uri: &str, format: &SourceFormat) -> String {
    let sql = crate::export::sql::render_sql(expr, uri, format);
    let escaped_sql = sql.replace('"', "\\\"");
    format!("duckdb -c \"{escaped_sql}\"")
}

pub fn render_shell_csv(expr: &ViewExpr, uri: &str, format: &SourceFormat) -> String {
    let sql = crate::export::sql::render_sql(expr, uri, format);
    let escaped_sql = sql.replace('"', "\\\"");
    format!("duckdb -c \"COPY ({escaped_sql}) TO 'output.csv' (FORMAT CSV, HEADER true)\"")
}
