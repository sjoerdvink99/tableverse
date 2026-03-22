use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, Query, State},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    Json,
};
use futures::{stream, StreamExt};
use serde::Deserialize;
use std::convert::Infallible;
use tv_engine::reader::RowGroupColumnStat;

#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    bins: Option<usize>,
}

#[derive(Deserialize)]
pub struct BatchRgStatsQuery {
    cols: String,
}

#[tracing::instrument(skip(state, params), fields(source_id = %id))]
pub async fn column_stats(
    State(state): State<AppState>,
    Path((id, col_idx)): Path<(String, usize)>,
    Query(params): Query<StatsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let n_bins = params.bins.unwrap_or(50).clamp(10, 512);

    if n_bins == 50 {
        let cache_key = format!("tt:stats:{id}:{col_idx}");
        if let Some(cached) = state.cache.get(&cache_key) {
            if let Ok(stats) = serde_json::from_slice::<serde_json::Value>(&cached) {
                return Ok(Json(stats));
            }
        }
        let stats = state.engine.column_stats(&id, col_idx, n_bins).await?;
        let json = serde_json::to_vec(&stats).map_err(|e| AppError::Internal(e.to_string()))?;
        state.cache.set(&cache_key, json.clone(), 300);
        let value: serde_json::Value =
            serde_json::from_slice(&json).map_err(|e| AppError::Internal(e.to_string()))?;
        return Ok(Json(value));
    }

    let stats = state.engine.column_stats(&id, col_idx, n_bins).await?;
    let json = serde_json::to_vec(&stats).map_err(|e| AppError::Internal(e.to_string()))?;
    let value: serde_json::Value =
        serde_json::from_slice(&json).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(value))
}

pub async fn profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let cache_key = format!("tt:profile:{id}");

    if let Some(cached) = state.cache.get(&cache_key) {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&cached) {
            return Ok(Json(v));
        }
    }

    let stats = state.engine.profile_source(&id).await?;
    let json = serde_json::to_vec(&stats).map_err(|e| AppError::Internal(e.to_string()))?;
    state.cache.set(&cache_key, json.clone(), 300);
    let value: serde_json::Value =
        serde_json::from_slice(&json).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(value))
}

pub async fn row_group_stats(
    State(state): State<AppState>,
    Path((id, col_idx)): Path<(String, usize)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let cache_key = format!("tt:rgs:{id}:{col_idx}");

    if let Some(cached) = state.cache.get(&cache_key) {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&cached) {
            return Ok(Json(v));
        }
    }

    let stats: Vec<RowGroupColumnStat> = state
        .engine
        .row_group_stats(&id, col_idx)
        .await
        .map_err(AppError::Engine)?;

    let value = serde_json::to_value(&stats).map_err(|e| AppError::Internal(e.to_string()))?;
    let bytes = serde_json::to_vec(&value).map_err(|e| AppError::Internal(e.to_string()))?;
    state.cache.set(&cache_key, bytes, 600);
    Ok(Json(value))
}

pub async fn row_group_stats_batch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<BatchRgStatsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let col_indices: Vec<usize> = params
        .cols
        .split(',')
        .filter_map(|s| s.trim().parse::<usize>().ok())
        .collect();

    let mut result = serde_json::Map::new();

    for col_idx in col_indices {
        let cache_key = format!("tt:rgs:{id}:{col_idx}");

        let stats: Vec<RowGroupColumnStat> = if let Some(cached) = state.cache.get(&cache_key) {
            serde_json::from_slice(&cached).unwrap_or_default()
        } else {
            let s = state
                .engine
                .row_group_stats(&id, col_idx)
                .await
                .map_err(AppError::Engine)?;
            let bytes = serde_json::to_vec(&s).map_err(|e| AppError::Internal(e.to_string()))?;
            state.cache.set(&cache_key, bytes, 600);
            s
        };

        let v = serde_json::to_value(&stats).map_err(|e| AppError::Internal(e.to_string()))?;
        result.insert(col_idx.to_string(), v);
    }

    Ok(Json(serde_json::Value::Object(result)))
}

pub async fn correlations(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let cache_key = format!("tt:corr:{id}");

    if let Some(cached) = state.cache.get(&cache_key) {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&cached) {
            return Ok(Json(v));
        }
    }

    let matrix = state.engine.correlations(&id).await?;
    let json = serde_json::to_vec(&matrix).map_err(|e| AppError::Internal(e.to_string()))?;
    state.cache.set(&cache_key, json.clone(), 300);
    let value: serde_json::Value =
        serde_json::from_slice(&json).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(value))
}

#[tracing::instrument(skip(state, params), fields(source_id = %id))]
pub async fn column_stats_stream(
    State(state): State<AppState>,
    Path((id, col_idx)): Path<(String, usize)>,
    Query(params): Query<StatsQuery>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, AppError> {
    let source = state
        .engine
        .get_source(&id)
        .ok_or_else(|| AppError::NotFound(id.clone()))?;

    let n_bins = params.bins.unwrap_or(50).clamp(10, 512);

    let quick = source.quick_stats.get(col_idx).cloned();
    let metadata_json = serde_json::json!({
        "min": quick.as_ref().and_then(|q| q.min.clone()),
        "max": quick.as_ref().and_then(|q| q.max.clone()),
        "null_count": quick.as_ref().map(|q| q.null_count).unwrap_or(0),
        "row_count": source.n_rows,
        "col_name": source.columns.get(col_idx).map(|c| c.name.clone()),
    });

    let state_coarse = state.clone();
    let id_coarse = id.clone();
    let state_full = state.clone();
    let id_full = id.clone();

    let s = stream::iter(vec![Ok(Event::default()
        .event("metadata")
        .data(metadata_json.to_string()))])
    .chain(stream::once(async move {
        let coarse = state_coarse
            .engine
            .column_stats_coarse(&id_coarse, col_idx)
            .await;
        match coarse {
            Ok(s) => {
                let json = serde_json::to_value(&s)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| "{}".to_string());
                Ok(Event::default().event("histogram_coarse").data(json))
            }
            Err(_) => Ok(Event::default().event("histogram_coarse").data("{}")),
        }
    }))
    .chain(stream::once(async move {
        let stats = state_full
            .engine
            .column_stats(&id_full, col_idx, n_bins)
            .await;
        match stats {
            Ok(s) => {
                let json = serde_json::to_value(&s)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                Ok(Event::default().event("stats").data(json))
            }
            Err(e) => Ok(Event::default().event("error").data(e.to_string())),
        }
    }))
    .chain(stream::iter(vec![Ok(Event::default()
        .event("done")
        .data("{}"))]));

    Ok(Sse::new(s))
}
