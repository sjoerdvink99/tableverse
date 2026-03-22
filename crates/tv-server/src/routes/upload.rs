use crate::{error::AppError, state::AppState};
use axum::{
    body::Bytes,
    extract::{Request, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

#[tracing::instrument(skip(state, req))]
pub async fn upload_source(
    State(state): State<AppState>,
    req: Request,
) -> Result<impl IntoResponse, AppError> {
    let name = req
        .headers()
        .get("x-tv-name")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let content_type = req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let is_parquet = content_type.contains("parquet")
        || req
            .headers()
            .get("x-tv-format")
            .and_then(|v| v.to_str().ok())
            .map(|s| s == "parquet")
            .unwrap_or(false);

    let body: Bytes = axum::body::to_bytes(req.into_body(), 4 * 1024 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    if body.is_empty() {
        return Err(AppError::BadRequest("empty upload body".to_string()));
    }

    let meta = state.engine.register_upload(body, name, is_parquet).await?;

    Ok((StatusCode::CREATED, Json(meta)))
}
