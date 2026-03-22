use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("source not found: {0}")]
    SourceNotFound(String),

    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("query error: {0}")]
    Query(String),

    #[error("arrow: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    #[error("parquet: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),

    #[error("core error: {0}")]
    Core(#[from] tv_core::CoreError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid cast: column {col} cannot be cast to {expected}")]
    InvalidCast { col: String, expected: &'static str },

    #[error("internal: {0}")]
    Internal(String),
}
