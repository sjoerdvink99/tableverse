use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("source not found: {0}")]
    SourceNotFound(String),

    #[error("invalid tile request: {0}")]
    InvalidTileRequest(String),

    #[error("arrow error: {0}")]
    Arrow(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
