use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;
use tracing;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("engine error: {0}")]
    Engine(#[from] tv_engine::error::EngineError),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("request timed out")]
    Timeout,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Engine(e) => match e {
                tv_engine::error::EngineError::SourceNotFound(id) => {
                    (StatusCode::NOT_FOUND, format!("source not found: {id}"))
                }
                tv_engine::error::EngineError::UnsupportedFormat(msg) => {
                    (StatusCode::BAD_REQUEST, msg.clone())
                }
                tv_engine::error::EngineError::Query(msg) if msg == "source_modified" => {
                    return (
                        StatusCode::CONFLICT,
                        Json(json!({ "reason": "source_modified" })),
                    )
                        .into_response();
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            },
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            AppError::Timeout => (StatusCode::REQUEST_TIMEOUT, "request timed out".to_string()),
        };

        if status.is_server_error() {
            tracing::error!(status = status.as_u16(), error = %message, "request failed");
        } else if status.is_client_error() && status != StatusCode::NOT_FOUND {
            tracing::warn!(status = status.as_u16(), error = %message, "bad request");
        }

        (status, Json(json!({ "error": message }))).into_response()
    }
}
