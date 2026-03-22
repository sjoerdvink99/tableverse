pub mod delta;
pub mod error;
pub mod glue;
pub mod huggingface;
pub mod iceberg;

pub use error::IntegrationError;

#[derive(Debug, Clone)]
pub struct ResolvedSource {
    pub parquet_uris: Vec<String>,
    pub snapshot_id: Option<String>,
    pub source_format: ResolvedFormat,
}

#[derive(Debug, Clone)]
pub enum ResolvedFormat {
    Parquet,
    DeltaParquet,
    IcebergParquet,
}

pub async fn resolve(
    uri: &str,
    credentials: Option<&tv_core::Credentials>,
) -> Result<ResolvedSource, IntegrationError> {
    let lower = uri.to_lowercase();
    if lower.starts_with("delta://") || lower.starts_with("delta+s3://") {
        delta::resolve(uri, credentials).await
    } else if lower.starts_with("iceberg://") {
        iceberg::resolve(uri, credentials).await
    } else if lower.starts_with("hf://") || lower.starts_with("huggingface://") {
        huggingface::resolve(uri, credentials).await
    } else if lower.starts_with("glue://") {
        glue::resolve(uri, credentials).await
    } else {
        Err(IntegrationError::InvalidUri(format!(
            "unsupported URI scheme: {uri}"
        )))
    }
}
