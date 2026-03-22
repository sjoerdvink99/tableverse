use crate::state::AppState;
use axum::{extract::State, response::IntoResponse, Json};
use serde_json::json;

pub async fn get_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let source_count = state.engine.list_sources().len();
    let job_count = state.job_registry.job_count().await;
    let cache_bytes = state.cache.byte_size();
    let cache_entries = state.cache.entry_count();
    Json(json!({
        "sources_count": source_count,
        "job_count": job_count,
        "tile_cache_bytes": cache_bytes,
        "tile_cache_entries": cache_entries,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
