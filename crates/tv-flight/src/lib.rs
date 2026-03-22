pub mod client;
pub mod descriptor;
pub mod error;
pub mod service;

use std::net::SocketAddr;
use std::sync::Arc;

use arrow_flight::flight_service_server::FlightServiceServer;
use tonic::transport::Server;
use tracing::info;

pub use client::{parse_flight_uri, FlightClient};
pub use error::FlightError;
pub use service::TableverseFlightService;

pub struct FlightServerConfig {
    pub port: u16,
}

pub async fn serve(
    engine: Arc<tv_engine::Engine>,
    config: FlightServerConfig,
) -> Result<(), FlightError> {
    let addr: SocketAddr = format!("0.0.0.0:{}", config.port)
        .parse()
        .map_err(|_| FlightError::InvalidDescriptor(format!("invalid port: {}", config.port)))?;
    let service = TableverseFlightService::new(engine);

    info!(port = config.port, addr = %addr, "Arrow Flight server listening");

    Server::builder()
        .add_service(FlightServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
