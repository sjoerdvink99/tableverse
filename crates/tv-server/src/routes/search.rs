use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use tv_core::SearchResults;

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub columns: Option<Vec<String>>,
    pub limit: Option<usize>,
}

pub async fn search(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SearchRequest>,
) -> Result<impl IntoResponse, AppError> {
    let limit = req.limit.unwrap_or(100);
    let rows = state
        .engine
        .search(&id, &req.query, req.columns, limit)
        .await?;
    let total = rows.len();
    Ok(Json(SearchResults { rows, total }))
}
