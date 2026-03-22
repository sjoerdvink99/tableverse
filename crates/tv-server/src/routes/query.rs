use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;
use tv_core::{ViewExpr, DEFAULT_TILE_COLS, DEFAULT_TILE_ROWS};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileMode {
    Raw,
    Agg,
}

#[derive(Debug, Deserialize)]
pub struct TileBody {
    pub view_expr: ViewExpr,
    pub row: Option<u64>,
    pub col: Option<usize>,
    pub rows: Option<u64>,
    pub cols: Option<usize>,
    pub mode: Option<TileMode>,
}

#[derive(Debug, Deserialize)]
pub struct ViewBody {
    pub view_expr: ViewExpr,
}

#[derive(Debug, Serialize)]
pub struct CountResponse {
    count: u64,
}

#[tracing::instrument(skip(state), fields(source_id = %_id))]
pub async fn query_tile(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Json(body): Json<TileBody>,
) -> Result<impl IntoResponse, AppError> {
    let start = std::time::Instant::now();
    let row = body.row.unwrap_or(0);
    let col = body.col.unwrap_or(0);
    let rows = body.rows.unwrap_or(DEFAULT_TILE_ROWS);
    let cols = body.cols.unwrap_or(DEFAULT_TILE_COLS);

    let view_hash = tv_engine::Engine::ops_view_hash(&body.view_expr);
    let tile_row = row / rows;
    let tile_col = col / cols;
    let cache_key = tv_core::tile_key(&body.view_expr.source_id, &view_hash, tile_row, tile_col);

    if let Some(cached) = state.cache.get(&cache_key) {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/vnd.apache.arrow.stream"),
        );
        headers.insert("x-tv-tile-status", HeaderValue::from_static("exact"));
        return Ok((headers, cached));
    }

    let resp = if matches!(body.mode, Some(TileMode::Agg)) {
        timeout(
            Duration::from_secs(120),
            state
                .engine
                .query_view_tile_agg(&body.view_expr, row, col, rows, cols),
        )
        .await
        .map_err(|_| AppError::Timeout)?
        .map_err(AppError::Engine)?
    } else {
        timeout(
            Duration::from_secs(120),
            state
                .engine
                .query_view_tile(&body.view_expr, row, col, rows, cols),
        )
        .await
        .map_err(|_| AppError::Timeout)?
        .map_err(AppError::Engine)?
    };

    let is_provisional = resp.is_provisional;
    let job_id = resp.job_id.clone();

    if !is_provisional {
        state
            .cache
            .set(&cache_key, resp.data.clone(), state.tile_cache_ttl);
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.apache.arrow.stream"),
    );
    headers.insert(
        "x-tv-tile-status",
        HeaderValue::from_static(if is_provisional {
            "provisional"
        } else {
            "exact"
        }),
    );
    if let Some(jid) = job_id {
        if let Ok(v) = HeaderValue::from_str(&jid) {
            headers.insert("x-tv-job-id", v);
        }
    }

    let elapsed = start.elapsed();
    if elapsed.as_millis() > 5000 {
        tracing::warn!(elapsed_ms = elapsed.as_millis(), source_id = %_id, "slow tile query");
    }

    Ok((headers, resp.data))
}

#[derive(Debug, Deserialize)]
pub struct BatchTileBody {
    pub view_expr: ViewExpr,
    pub tiles: Vec<tv_core::BatchTileRequest>,
}

#[tracing::instrument(skip(state), fields(source_id = %_id))]
pub async fn query_tiles_batch(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Json(body): Json<BatchTileBody>,
) -> axum::response::Response {
    use axum::body::Body;
    use futures::stream::FuturesUnordered;
    use futures::StreamExt;
    use std::convert::Infallible;
    let start = std::time::Instant::now();

    let view_hash = tv_engine::Engine::ops_view_hash(&body.view_expr);
    let expr = body.view_expr;
    let tiles = body.tiles;
    let engine = state.engine.clone();
    let cache = state.cache.clone();
    let ttl = state.tile_cache_ttl;

    let futs: FuturesUnordered<_> = tiles
        .into_iter()
        .enumerate()
        .map(|(idx, t)| {
            let engine = engine.clone();
            let cache = cache.clone();
            let expr = expr.clone();
            let view_hash = view_hash.clone();
            async move {
                let tile_row = t.row / t.rows.max(1);
                let tile_col = t.col / t.cols.max(1);
                let cache_key = tv_core::tile_key(&expr.source_id, &view_hash, tile_row, tile_col);
                let ipc = if let Some(cached) = cache.get(&cache_key) {
                    cached
                } else {
                    match engine
                        .query_view_tile(&expr, t.row, t.col, t.rows, t.cols)
                        .await
                    {
                        Ok(resp) => {
                            if !resp.is_provisional {
                                cache.set(&cache_key, resp.data.clone(), ttl);
                            }
                            resp.data
                        }
                        Err(_) => vec![],
                    }
                };
                (idx, ipc)
            }
        })
        .collect();

    let stream = futs.map(|(idx, ipc): (usize, Vec<u8>)| {
        let mut chunk = Vec::with_capacity(8 + ipc.len());
        chunk.extend_from_slice(&(idx as u32).to_le_bytes());
        chunk.extend_from_slice(&(ipc.len() as u32).to_le_bytes());
        chunk.extend_from_slice(&ipc);
        Ok::<_, Infallible>(axum::body::Bytes::from(chunk))
    });

    let elapsed = start.elapsed();
    if elapsed.as_millis() > 5000 {
        tracing::warn!(elapsed_ms = elapsed.as_millis(), source_id = %_id, "slow tile batch query");
    }

    axum::response::Response::builder()
        .header("content-type", "application/octet-stream")
        .body(Body::from_stream(stream))
        .unwrap()
}

#[tracing::instrument(skip(state), fields(source_id = %_id))]
pub async fn query_count(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Json(body): Json<ViewBody>,
) -> Result<Json<CountResponse>, AppError> {
    let count = timeout(
        Duration::from_secs(120),
        state.engine.query_view_count(&body.view_expr),
    )
    .await
    .map_err(|_| AppError::Timeout)??;
    Ok(Json(CountResponse { count }))
}

pub async fn query_schema(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Json(body): Json<ViewBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let columns = state.engine.query_view_schema(&body.view_expr)?;
    Ok(Json(serde_json::json!({ "columns": columns })))
}
