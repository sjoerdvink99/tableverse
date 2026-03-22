use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, Query, State},
    http::header,
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Deserialize;
use tv_core::ViewExpr;
use tv_engine::{CodegenTarget, DownloadFormat};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Sql,
    AnsiSql,
    PythonDuckdb,
    PythonPolars,
    PythonPandas,
    Shell,
    ShellCsv,
    Dbt,
}

#[derive(Debug, Deserialize)]
pub struct ExportBody {
    pub view_expr: ViewExpr,
    pub format: ExportFormat,
}

#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    pub format: DownloadFormat,
    pub view_expr: String,
}

pub async fn export_code(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Json(body): Json<ExportBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let target = match body.format {
        ExportFormat::Sql => CodegenTarget::DuckdbSql,
        ExportFormat::AnsiSql => CodegenTarget::AnsiSql,
        ExportFormat::PythonDuckdb => CodegenTarget::PythonDuckdb,
        ExportFormat::PythonPolars => CodegenTarget::PythonPolars,
        ExportFormat::PythonPandas => CodegenTarget::PythonPandas,
        ExportFormat::Shell => CodegenTarget::Shell,
        ExportFormat::ShellCsv => CodegenTarget::ShellCsv,
        ExportFormat::Dbt => CodegenTarget::Dbt,
    };
    let code = state.engine.codegen(&body.view_expr, target)?;
    Ok(Json(serde_json::json!({ "code": code })))
}

pub async fn download(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Query(q): Query<DownloadQuery>,
) -> Result<impl IntoResponse, AppError> {
    let json_bytes = STANDARD
        .decode(&q.view_expr)
        .map_err(|_| AppError::BadRequest("invalid base64 encoding".into()))?;
    let view_expr: ViewExpr = serde_json::from_slice(&json_bytes)
        .map_err(|e| AppError::BadRequest(format!("invalid view_expr: {e}")))?;
    let (data, content_type) = state.engine.download_view(&view_expr, q.format).await?;
    Ok(([(header::CONTENT_TYPE, content_type)], data))
}
