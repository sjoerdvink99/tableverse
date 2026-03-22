pub mod assets;
pub mod cache;
pub mod error;
pub mod routes;
pub mod state;

use axum::http::HeaderValue;
use std::net::SocketAddr;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::{info, Level};

pub struct ServerConfig {
    pub port: u16,
    pub redis_url: Option<String>,
}

pub async fn serve(engine: tv_engine::Engine, config: ServerConfig) -> anyhow::Result<()> {
    let state = state::AppState::new(engine, config.redis_url);

    let allowed_origins = std::env::var("ALLOWED_ORIGINS")
        .map(|origins| {
            let headers: Vec<HeaderValue> = origins
                .split(',')
                .filter_map(|o| o.trim().parse().ok())
                .collect();
            if headers.is_empty() {
                AllowOrigin::any()
            } else {
                AllowOrigin::list(headers)
            }
        })
        .unwrap_or_else(|_| AllowOrigin::any());

    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = routes::router(state)
        .layer(
            TraceLayer::new_for_http()
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(
                    DefaultOnResponse::new()
                        .level(Level::INFO)
                        .latency_unit(tower_http::LatencyUnit::Millis),
                )
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
        .layer(cors);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    info!(addr = %addr, "server listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
