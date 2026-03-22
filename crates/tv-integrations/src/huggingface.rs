use crate::{IntegrationError, ResolvedFormat, ResolvedSource};
use serde::Deserialize;
use tv_core::Credentials;

const HF_API_BASE: &str = "https://huggingface.co";

#[derive(Debug, Deserialize)]
struct HfParquetFile {
    url: String,
}

pub async fn resolve(
    uri: &str,
    credentials: Option<&Credentials>,
) -> Result<ResolvedSource, IntegrationError> {
    let rest = uri
        .strip_prefix("hf://datasets/")
        .or_else(|| uri.strip_prefix("huggingface://datasets/"))
        .ok_or_else(|| {
            IntegrationError::InvalidUri(
                "HuggingFace URI must start with hf://datasets/ or huggingface://datasets/"
                    .to_string(),
            )
        })?;

    let parts: Vec<&str> = rest.splitn(3, '/').collect();
    let (owner, name, split) = match parts.as_slice() {
        [owner, name] => (*owner, *name, "train"),
        [owner, name, split] => (*owner, *name, *split),
        _ => {
            return Err(IntegrationError::InvalidUri(
                "hf://datasets/{owner}/{name}[/{split}]".to_string(),
            ))
        }
    };

    let client = build_client(credentials)?;
    let url = format!("{HF_API_BASE}/api/datasets/{owner}/{name}/parquet/{split}");

    let files: Vec<HfParquetFile> = client
        .get(&url)
        .send()
        .await?
        .error_for_status()
        .map_err(|e| IntegrationError::NotFound(format!("dataset {owner}/{name}/{split}: {e}")))?
        .json()
        .await?;

    if files.is_empty() {
        return Err(IntegrationError::NotFound(format!(
            "no Parquet files found for {owner}/{name}/{split}"
        )));
    }

    let parquet_uris: Vec<String> = files.into_iter().map(|f| f.url).collect();

    Ok(ResolvedSource {
        parquet_uris,
        snapshot_id: None,
        source_format: ResolvedFormat::Parquet,
    })
}

fn build_client(credentials: Option<&Credentials>) -> Result<reqwest::Client, IntegrationError> {
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60));

    if let Some(creds) = credentials {
        if let Some(token) = &creds.access_key {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|e| IntegrationError::Resolution(e.to_string()))?,
            );
            builder = builder.default_headers(headers);
        }
    }

    builder.build().map_err(IntegrationError::Network)
}
