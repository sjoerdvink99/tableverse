use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use serde_json::json;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::{error::AppError, state::AppState};

pub async fn job_events(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let job = state
        .job_registry
        .get_job(&job_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("job not found: {job_id}")))?;

    let rx = job.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        result.ok().map(|event| {
            let data = serde_json::to_string(&event).unwrap_or_default();
            let event_name = match &event {
                tv_engine::job_registry::JobEvent::TileReady { .. } => "tile_ready",
                tv_engine::job_registry::JobEvent::Progress { .. } => "progress",
                tv_engine::job_registry::JobEvent::Complete { .. } => "job_complete",
                tv_engine::job_registry::JobEvent::Failed { .. } => "job_failed",
            };
            Ok::<Event, Infallible>(Event::default().event(event_name).data(data))
        })
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

pub async fn get_job_status(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let job = state
        .job_registry
        .get_job(&job_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("job not found: {job_id}")))?;

    let phase = job.phase.read().await;
    let status = match &*phase {
        tv_engine::job_registry::JobPhase::Sampling => json!({ "phase": "sampling" }),
        tv_engine::job_registry::JobPhase::FullScan {
            rows_processed,
            total_rows,
        } => json!({
            "phase": "full_scan",
            "rows_processed": rows_processed,
            "total_rows": total_rows,
        }),
        tv_engine::job_registry::JobPhase::Complete => json!({ "phase": "complete" }),
        tv_engine::job_registry::JobPhase::Failed(msg) => json!({
            "phase": "failed",
            "message": msg,
        }),
    };

    Ok(Json(status))
}
