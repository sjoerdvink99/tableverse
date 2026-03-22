use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, Clone)]
pub enum AuthMode {
    None,
    ApiKey(String),
}

impl AuthMode {
    pub fn from_env() -> Self {
        if let Ok(key) = std::env::var("TV_API_KEY") {
            if !key.is_empty() {
                return AuthMode::ApiKey(key);
            }
        }
        AuthMode::None
    }
}

pub async fn auth_middleware(request: Request, next: Next) -> Result<Response, Response> {
    let mode = AuthMode::from_env();

    match mode {
        AuthMode::None => Ok(next.run(request).await),
        AuthMode::ApiKey(expected_key) => {
            let path = request.uri().path().to_string();

            if path == "/healthz" || path.starts_with("/assets/") {
                return Ok(next.run(request).await);
            }

            let provided = request
                .headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .or_else(|| {
                    request
                        .headers()
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.strip_prefix("Bearer "))
                });

            match provided {
                Some(key) if key == expected_key => Ok(next.run(request).await),
                _ => Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "unauthorized" })),
                )
                    .into_response()),
            }
        }
    }
}
