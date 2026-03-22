pub mod export;
pub mod health;
pub mod jobs;
pub mod metrics;
pub mod optimize;
pub mod profiles;
pub mod query;
pub mod search;
pub mod sources;
pub mod stats;
pub mod tiles;

use crate::assets::serve_asset;
use crate::state::AppState;
use axum::{
    routing::{delete, get, post},
    Router,
};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/v1/sources", get(sources::list_sources))
        .route("/api/v1/sources", post(sources::register_source))
        .route("/api/v1/sources/{id}", get(sources::get_source))
        .route("/api/v1/sources/{id}", delete(sources::delete_source))
        .route("/api/v1/sources/{id}/tiles", get(tiles::get_tile))
        .route(
            "/api/v1/sources/{id}/columns/{col_idx}/stats",
            get(stats::column_stats),
        )
        .route(
            "/api/v1/sources/{id}/columns/{col_idx}/stats/stream",
            get(stats::column_stats_stream),
        )
        .route(
            "/api/v1/sources/{id}/columns/{col_idx}/row-group-stats",
            get(stats::row_group_stats),
        )
        .route(
            "/api/v1/sources/{id}/row-group-stats/batch",
            get(stats::row_group_stats_batch),
        )
        .route("/api/v1/sources/{id}/profile", get(stats::profile))
        .route(
            "/api/v1/sources/{id}/correlations",
            get(stats::correlations),
        )
        .route("/api/v1/sources/{id}/search", post(search::search))
        .route("/api/v1/sources/{id}/query/tiles", post(query::query_tile))
        .route(
            "/api/v1/sources/{id}/query/tiles/batch",
            post(query::query_tiles_batch),
        )
        .route("/api/v1/sources/{id}/query/count", post(query::query_count))
        .route(
            "/api/v1/sources/{id}/query/schema",
            post(query::query_schema),
        )
        .route(
            "/api/v1/sources/{id}/query/export",
            post(export::export_code),
        )
        .route("/api/v1/sources/{id}/query/download", get(export::download))
        .route("/api/v1/profiles", get(profiles::list_profiles))
        .route("/api/v1/jobs/{id}/events", get(jobs::job_events))
        .route("/api/v1/jobs/{id}", get(jobs::get_job_status))
        .route("/api/v1/metrics", get(metrics::get_metrics))
        .route(
            "/api/v1/sources/{id}/optimize",
            post(optimize::optimize_source),
        )
        .fallback(serve_asset)
        .with_state(state)
}
