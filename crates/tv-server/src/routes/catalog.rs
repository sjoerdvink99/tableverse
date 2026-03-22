use crate::{error::AppError, state::AppState};
use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct BrowseRequest {
    #[serde(rename = "type")]
    pub catalog_type: String,
    pub endpoint: Option<String>,
    pub warehouse: Option<String>,
    pub namespace: Option<String>,
    pub dataset: Option<String>,
    pub split: Option<String>,
    pub database: Option<String>,
    pub table: Option<String>,
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub prefix: Option<String>,
    pub token: Option<String>,
    pub host: Option<String>,
    pub query: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BrowseEntry {
    pub name: String,
    pub namespace: String,
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_rows: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BrowseResponse {
    pub entries: Vec<BrowseEntry>,
}

pub async fn browse_catalog(
    State(_state): State<AppState>,
    Json(req): Json<BrowseRequest>,
) -> Result<impl IntoResponse, AppError> {
    let entries = match req.catalog_type.as_str() {
        "huggingface" => browse_huggingface(&req).await?,
        "delta" => browse_delta(&req)?,
        "iceberg_rest" => browse_iceberg(&req).await?,
        "s3" => browse_s3(&req).await?,
        "glue" => browse_glue(&req)?,
        "clickhouse" => browse_clickhouse(&req).await?,
        other => {
            return Err(AppError::BadRequest(format!(
                "unsupported catalog type: {other}"
            )))
        }
    };

    Ok(Json(BrowseResponse { entries }))
}

async fn browse_huggingface(req: &BrowseRequest) -> Result<Vec<BrowseEntry>, AppError> {
    let dataset = req
        .dataset
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("dataset field required".to_string()))?;

    let splits = ["train", "validation", "test"];
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut entries = Vec::new();
    for split in &splits {
        let url = format!("https://huggingface.co/api/datasets/{dataset}/parquet/{split}");
        let mut request = client.get(&url);
        if let Some(token) = &req.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if let Ok(response) = request.send().await {
            if response.status().is_success() {
                if let Ok(files) = response.json::<Vec<serde_json::Value>>().await {
                    if !files.is_empty() {
                        entries.push(BrowseEntry {
                            name: format!("{dataset}/{split}"),
                            namespace: dataset.to_string(),
                            uri: format!("hf://datasets/{dataset}/{split}"),
                            n_rows: None,
                            columns: vec![],
                        });
                    }
                }
            }
        }
    }

    if entries.is_empty() {
        entries.push(BrowseEntry {
            name: format!("{dataset}/train"),
            namespace: dataset.to_string(),
            uri: format!("hf://datasets/{dataset}/train"),
            n_rows: None,
            columns: vec![],
        });
    }

    Ok(entries)
}

fn browse_delta(req: &BrowseRequest) -> Result<Vec<BrowseEntry>, AppError> {
    let path = req
        .path
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("path field required for Delta".to_string()))?;

    let log_dir = std::path::Path::new(path).join("_delta_log");
    if !log_dir.exists() {
        return Err(AppError::NotFound(format!(
            "_delta_log not found at {path}"
        )));
    }

    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string();

    Ok(vec![BrowseEntry {
        name: name.clone(),
        namespace: path.to_string(),
        uri: format!("delta://{path}"),
        n_rows: None,
        columns: vec![],
    }])
}

async fn browse_iceberg(req: &BrowseRequest) -> Result<Vec<BrowseEntry>, AppError> {
    let endpoint = req
        .endpoint
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("endpoint required".to_string()))?;
    let warehouse = req
        .warehouse
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("warehouse required".to_string()))?;
    let namespace = req
        .namespace
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("namespace required".to_string()))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let url = format!("{endpoint}/v1/namespaces/{namespace}/tables");
    let mut request = client.get(&url).query(&[("warehouse", warehouse)]);
    if let Some(token) = &req.token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    let response = request
        .send()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if !response.status().is_success() {
        return Err(AppError::Internal(format!(
            "Iceberg catalog returned {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        )));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let tables = body
        .get("identifiers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let base = endpoint
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let entries = tables
        .iter()
        .filter_map(|t| {
            let table_name = t.get("name")?.as_str()?;
            let ns = t
                .get("namespace")
                .and_then(|n| n.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .unwrap_or(namespace);
            Some(BrowseEntry {
                name: table_name.to_string(),
                namespace: ns.to_string(),
                uri: format!("iceberg://{base}/{warehouse}/{ns}/{table_name}"),
                n_rows: None,
                columns: vec![],
            })
        })
        .collect();

    Ok(entries)
}

async fn browse_s3(req: &BrowseRequest) -> Result<Vec<BrowseEntry>, AppError> {
    let bucket = req
        .bucket
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("bucket required".to_string()))?;
    let prefix = req.prefix.as_deref().unwrap_or("");

    Ok(vec![BrowseEntry {
        name: format!("{bucket}/{prefix}"),
        namespace: bucket.to_string(),
        uri: format!("s3://{bucket}/{prefix}"),
        n_rows: None,
        columns: vec![],
    }])
}

fn browse_glue(_req: &BrowseRequest) -> Result<Vec<BrowseEntry>, AppError> {
    Err(AppError::BadRequest(
        "AWS Glue catalog browsing requires aws-sdk-glue; provide the S3 table path directly"
            .to_string(),
    ))
}

async fn browse_clickhouse(req: &BrowseRequest) -> Result<Vec<BrowseEntry>, AppError> {
    let host = req.host.as_deref().unwrap_or("localhost");
    let database = req
        .database
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("database required".to_string()))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let query = format!(
        "SELECT name, total_rows FROM system.tables WHERE database = '{}' FORMAT JSON",
        database.replace('\'', "")
    );

    let response = client
        .post(format!("http://{}:8123/", host))
        .body(query)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("ClickHouse connection failed: {e}")))?;

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let rows = body
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    let entries = rows
        .iter()
        .filter_map(|row| {
            let name = row.get("name")?.as_str()?;
            let n_rows = row
                .get("total_rows")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok());
            Some(BrowseEntry {
                name: name.to_string(),
                namespace: database.to_string(),
                uri: format!("clickhouse://{}:8123/{}", host, database),
                n_rows,
                columns: vec![],
            })
        })
        .collect();

    Ok(entries)
}
