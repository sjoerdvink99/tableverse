use crate::{IntegrationError, ResolvedSource};
use tv_core::Credentials;

pub async fn resolve(
    uri: &str,
    credentials: Option<&Credentials>,
) -> Result<ResolvedSource, IntegrationError> {
    let rest = uri.strip_prefix("glue://").unwrap_or(uri);

    let (path_part, _query) = rest.split_once('?').unwrap_or((rest, ""));
    let parts: Vec<&str> = path_part.splitn(2, '/').collect();

    if parts.len() < 2 {
        return Err(IntegrationError::InvalidUri(
            "Glue URI format: glue://{database}/{table}".to_string(),
        ));
    }

    let database = parts[0];
    let table = parts[1];

    if credentials.is_none()
        && std::env::var("AWS_ACCESS_KEY_ID").is_err()
        && std::env::var("AWS_PROFILE").is_err()
    {
        return Err(IntegrationError::Resolution(
            "AWS credentials required for Glue catalog access".to_string(),
        ));
    }

    tracing::info!(database = database, table = table, "resolving Glue table");

    Err(IntegrationError::Resolution(
        "AWS Glue integration requires aws-sdk-glue crate; provide S3 path directly or use the Python SDK with boto3".to_string(),
    ))
}
