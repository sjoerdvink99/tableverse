use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde_json::json;

pub async fn optimize_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    state
        .engine
        .optimize_source(&id)
        .await
        .map_err(AppError::Engine)?;
    Ok(Json(json!({ "status": "ok", "source_id": id })))
}
