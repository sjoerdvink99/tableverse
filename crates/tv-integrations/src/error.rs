use thiserror::Error;

#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("resolution failed: {0}")]
    Resolution(String),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("invalid uri: {0}")]
    InvalidUri(String),

    #[error("catalog error: {0}")]
    Catalog(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("serialization: {0}")]
    Serialization(#[from] serde_json::Error),
}
