use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlightError {
    #[error("engine error: {0}")]
    Engine(#[from] tv_engine::error::EngineError),

    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    #[error("transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    #[error("gRPC status: {0}")]
    Status(#[from] tonic::Status),

    #[error("invalid descriptor: {0}")]
    InvalidDescriptor(String),

    #[error("serialization: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<FlightError> for tonic::Status {
    fn from(e: FlightError) -> Self {
        match e {
            FlightError::NotFound(msg) => tonic::Status::not_found(msg),
            FlightError::InvalidDescriptor(msg) => tonic::Status::invalid_argument(msg),
            FlightError::Arrow(e) => tonic::Status::internal(format!("arrow: {e}")),
            FlightError::Engine(e) => tonic::Status::internal(format!("engine: {e}")),
            FlightError::Transport(e) => tonic::Status::unavailable(e.to_string()),
            FlightError::Io(e) => tonic::Status::unavailable(format!("io: {e}")),
            FlightError::Serialization(e) => tonic::Status::internal(format!("serialization: {e}")),
            FlightError::Status(s) => s,
        }
    }
}
