use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use tv_core::RegisterSourceRequest;

pub async fn list_sources(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    Ok(Json(state.engine.list_sources()))
}

#[tracing::instrument(skip(state, req))]
pub async fn register_source(
    State(state): State<AppState>,
    Json(req): Json<RegisterSourceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let meta = state
        .engine
        .register_source(&req.uri, req.name, req.profile, req.credentials)
        .await?;
    Ok((StatusCode::CREATED, Json(meta)))
}

pub async fn get_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let meta = state
        .engine
        .get_source(&id)
        .ok_or_else(|| AppError::NotFound(format!("source {id}")))?;
    Ok(Json(meta))
}

#[tracing::instrument(skip(state), fields(source_id = %id))]
pub async fn delete_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    state.engine.remove_source(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
