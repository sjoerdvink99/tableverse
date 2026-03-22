use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, Query, State},
    http::header,
    response::IntoResponse,
};
use serde::Deserialize;
use tv_core::{view_hash, FilterExpr, SortSpec, TileRequest, DEFAULT_TILE_COLS, DEFAULT_TILE_ROWS};

#[derive(Debug, Deserialize)]
pub struct TileQuery {
    pub row: Option<u64>,
    pub col: Option<usize>,
    pub rows: Option<u64>,
    pub cols: Option<usize>,
    pub sort_col: Option<String>,
    pub sort_desc: Option<bool>,
    pub filter: Option<String>,
}

pub async fn get_tile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TileQuery>,
) -> Result<impl IntoResponse, AppError> {
    let row = q.row.unwrap_or(0);
    let col = q.col.unwrap_or(0);
    let rows = q.rows.unwrap_or(DEFAULT_TILE_ROWS);
    let cols = q.cols.unwrap_or(DEFAULT_TILE_COLS);

    let sort = q.sort_col.map(|column| SortSpec {
        column,
        descending: q.sort_desc.unwrap_or(false),
    });

    let filter: Option<FilterExpr> = q
        .filter
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str(s).ok());

    let canonical = format!(
        "{}|{}",
        sort.as_ref()
            .map(|s| format!("{}:{}", s.column, s.descending))
            .unwrap_or_default(),
        filter
            .as_ref()
            .and_then(|f| serde_json::to_string(f).ok())
            .unwrap_or_default(),
    );
    let vh = view_hash(&canonical);
    let cache_key = tv_core::tile_key(&id, &vh, row / rows, col / cols);

    if let Some(cached) = state.cache.get(&cache_key) {
        return Ok((
            [(header::CONTENT_TYPE, "application/vnd.apache.arrow.stream")],
            cached,
        ));
    }

    let req = TileRequest {
        source_id: id.clone(),
        row,
        col,
        rows,
        cols,
        sort,
        filter,
    };

    let resp = state.engine.query_tile(&req).await?;
    state
        .cache
        .set(&cache_key, resp.data.clone(), state.tile_cache_ttl);

    Ok((
        [(header::CONTENT_TYPE, "application/vnd.apache.arrow.stream")],
        resp.data,
    ))
}
