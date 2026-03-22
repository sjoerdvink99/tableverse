use crate::{IntegrationError, ResolvedFormat, ResolvedSource};
use serde::Deserialize;
use tv_core::Credentials;

#[derive(Debug, Deserialize)]
struct IcebergTableResponse {
    #[serde(rename = "metadata-location")]
    metadata_location: String,
}

#[derive(Debug, Deserialize)]
struct IcebergMetadata {
    #[serde(rename = "current-snapshot-id")]
    current_snapshot_id: Option<i64>,
    snapshots: Option<Vec<IcebergSnapshot>>,
}

#[derive(Debug, Deserialize)]
struct IcebergSnapshot {
    #[serde(rename = "snapshot-id")]
    snapshot_id: i64,
    #[serde(rename = "manifest-list")]
    manifest_list: Option<String>,
}

pub async fn resolve(
    uri: &str,
    credentials: Option<&Credentials>,
) -> Result<ResolvedSource, IntegrationError> {
    let rest = uri.strip_prefix("iceberg://").unwrap_or(uri);

    if rest.starts_with('/') || rest.ends_with(".json") {
        resolve_from_metadata_file(rest, credentials).await
    } else {
        resolve_from_rest_catalog(rest, credentials).await
    }
}

async fn resolve_from_rest_catalog(
    path: &str,
    credentials: Option<&Credentials>,
) -> Result<ResolvedSource, IntegrationError> {
    let parts: Vec<&str> = path.splitn(4, '/').collect();
    if parts.len() < 4 {
        return Err(IntegrationError::InvalidUri(
            "iceberg:// URI requires format: iceberg://{endpoint}/{warehouse}/{namespace}/{table}"
                .to_string(),
        ));
    }

    let endpoint = format!("https://{}", parts[0]);
    let _warehouse = parts[1];
    let namespace = parts[2];
    let table = parts[3];

    let client = build_client(credentials)?;
    let url = format!("{endpoint}/v1/namespaces/{namespace}/tables/{table}");

    let response: IcebergTableResponse = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    resolve_from_metadata_url(&response.metadata_location, &client).await
}

async fn resolve_from_metadata_file(
    path: &str,
    _credentials: Option<&Credentials>,
) -> Result<ResolvedSource, IntegrationError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| IntegrationError::Resolution(e.to_string()))?;
    let metadata: IcebergMetadata = serde_json::from_str(&content)?;
    extract_parquet_files_from_metadata(&metadata, path).await
}

async fn resolve_from_metadata_url(
    metadata_location: &str,
    client: &reqwest::Client,
) -> Result<ResolvedSource, IntegrationError> {
    let content: IcebergMetadata = client
        .get(metadata_location)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    extract_parquet_files_from_metadata(&content, metadata_location).await
}

async fn extract_parquet_files_from_metadata(
    metadata: &IcebergMetadata,
    base_location: &str,
) -> Result<ResolvedSource, IntegrationError> {
    let snapshot_id = metadata.current_snapshot_id;

    let current_snapshot = metadata
        .snapshots
        .as_ref()
        .and_then(|snaps| snapshot_id.and_then(|id| snaps.iter().find(|s| s.snapshot_id == id)))
        .or_else(|| metadata.snapshots.as_ref()?.last());

    let snapshot = current_snapshot.ok_or_else(|| {
        IntegrationError::NotFound("no snapshot found in Iceberg metadata".to_string())
    })?;

    let manifest_list = snapshot
        .manifest_list
        .as_deref()
        .ok_or_else(|| IntegrationError::NotFound("snapshot has no manifest-list".to_string()))?;

    let base_dir = std::path::Path::new(base_location)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let manifest_list_path = if manifest_list.starts_with('/') || manifest_list.contains("://") {
        manifest_list.to_string()
    } else {
        format!("{}/{}", base_dir.trim_end_matches('/'), manifest_list)
    };

    let parquet_uris = collect_parquet_from_avro_manifest_list(&manifest_list_path).await?;

    Ok(ResolvedSource {
        parquet_uris,
        snapshot_id: snapshot_id.map(|id| id.to_string()),
        source_format: ResolvedFormat::IcebergParquet,
    })
}

async fn collect_parquet_from_avro_manifest_list(
    manifest_list_path: &str,
) -> Result<Vec<String>, IntegrationError> {
    if manifest_list_path.ends_with(".avro") || manifest_list_path.contains(".avro") {
        return Err(IntegrationError::Resolution(
            "Avro manifest lists require the apache-avro crate; use a REST catalog endpoint instead".to_string(),
        ));
    }

    if manifest_list_path.ends_with(".json") {
        let content = std::fs::read_to_string(manifest_list_path)
            .map_err(|e| IntegrationError::Resolution(e.to_string()))?;
        let entries: Vec<serde_json::Value> = serde_json::from_str(&content)?;
        let mut files: Vec<String> = Vec::new();
        for entry in entries {
            if let Some(path) = entry.get("file_path").and_then(|v| v.as_str()) {
                if path.ends_with(".parquet") {
                    files.push(path.to_string());
                }
            }
        }
        return Ok(files);
    }

    Err(IntegrationError::Resolution(format!(
        "unsupported manifest format: {manifest_list_path}"
    )))
}

fn build_client(credentials: Option<&Credentials>) -> Result<reqwest::Client, IntegrationError> {
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));

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
