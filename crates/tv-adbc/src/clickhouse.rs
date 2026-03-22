use arrow::ipc::reader::StreamReader;
use arrow::record_batch::RecordBatch;

use crate::{connection::ConnectionConfig, error::AdbcError};

pub async fn execute_query(
    config: &ConnectionConfig,
    query: &str,
) -> Result<Vec<RecordBatch>, AdbcError> {
    let url = build_url(config);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(AdbcError::Network)?;

    let arrow_query = format!("{} FORMAT Arrow", query.trim_end_matches(';'));

    let mut request = client.post(&url).body(arrow_query);

    if let Some(username) = &config.username {
        request = request.header("X-ClickHouse-User", username);
    }
    if let Some(password) = &config.password {
        request = request.header("X-ClickHouse-Key", password);
    }
    if !config.database.is_empty() {
        request = request.header("X-ClickHouse-Database", &config.database);
    }

    let response = request.send().await?.error_for_status()?;
    let bytes = response.bytes().await?;

    let reader = StreamReader::try_new(std::io::Cursor::new(bytes.as_ref()), None)?;
    let batches: Vec<RecordBatch> = reader.collect::<Result<_, _>>()?;

    Ok(batches)
}

pub async fn execute_tile(
    config: &ConnectionConfig,
    query: &str,
    row_offset: u64,
    row_limit: u64,
) -> Result<Vec<RecordBatch>, AdbcError> {
    let tile_query = format!(
        "SELECT * FROM ({}) LIMIT {} OFFSET {}",
        query.trim_end_matches(';'),
        row_limit,
        row_offset,
    );
    execute_query(config, &tile_query).await
}

pub async fn count_rows(config: &ConnectionConfig, query: &str) -> Result<u64, AdbcError> {
    let count_query = format!("SELECT count() FROM ({})", query.trim_end_matches(';'));
    let url = build_url(config);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(AdbcError::Network)?;

    let mut request = client
        .post(&url)
        .body(count_query)
        .header("Accept", "text/plain");

    if let Some(username) = &config.username {
        request = request.header("X-ClickHouse-User", username);
    }
    if let Some(password) = &config.password {
        request = request.header("X-ClickHouse-Key", password);
    }

    let text = request.send().await?.error_for_status()?.text().await?;
    text.trim()
        .parse::<u64>()
        .map_err(|e| AdbcError::Query(format!("count parse error: {e}: '{text}'")))
}

pub async fn infer_schema(
    config: &ConnectionConfig,
    query: &str,
) -> Result<arrow::datatypes::SchemaRef, AdbcError> {
    let limit_query = format!("SELECT * FROM ({}) LIMIT 1", query.trim_end_matches(';'));
    let batches = execute_query(config, &limit_query).await?;
    batches
        .first()
        .map(|b| b.schema())
        .ok_or_else(|| AdbcError::Query("empty result, cannot infer schema".to_string()))
}

fn build_url(config: &ConnectionConfig) -> String {
    let scheme = if config
        .extra_params
        .get("tls")
        .map(|s| s == "true")
        .unwrap_or(false)
    {
        "https"
    } else {
        "http"
    };
    format!("{}://{}:{}/", scheme, config.host, config.port)
}
