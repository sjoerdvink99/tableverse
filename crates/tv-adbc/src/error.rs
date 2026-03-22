use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdbcError {
    #[error("connection failed: {0}")]
    Connection(String),

    #[error("query failed: {0}")]
    Query(String),

    #[error("unsupported database: {0}")]
    Unsupported(String),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    #[error("serialization: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid connection string: {0}")]
    InvalidConnectionString(String),
}

impl From<AdbcError> for tv_engine::error::EngineError {
    fn from(e: AdbcError) -> Self {
        tv_engine::error::EngineError::Query(e.to_string())
    }
}
