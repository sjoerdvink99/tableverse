pub mod clickhouse;
pub mod connection;
pub mod error;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;

pub use connection::{ConnectionConfig, DatabaseDriver};
pub use error::AdbcError;

pub struct AdbcSource {
    pub config: ConnectionConfig,
    pub query: String,
}

impl AdbcSource {
    pub fn new(connection_string: &str, query: &str) -> Result<Self, AdbcError> {
        let config = ConnectionConfig::from_uri(connection_string)?;
        Ok(Self {
            config,
            query: query.to_string(),
        })
    }
}

pub async fn execute_query(source: &AdbcSource) -> Result<Vec<RecordBatch>, AdbcError> {
    dispatch_query(&source.config, &source.query).await
}

pub async fn execute_tile(
    source: &AdbcSource,
    row_offset: u64,
    row_limit: u64,
) -> Result<Vec<RecordBatch>, AdbcError> {
    dispatch_tile(&source.config, &source.query, row_offset, row_limit).await
}

pub async fn count_rows(source: &AdbcSource) -> Result<u64, AdbcError> {
    dispatch_count(&source.config, &source.query).await
}

pub async fn infer_schema(source: &AdbcSource) -> Result<SchemaRef, AdbcError> {
    dispatch_schema(&source.config, &source.query).await
}

async fn dispatch_query(
    config: &ConnectionConfig,
    query: &str,
) -> Result<Vec<RecordBatch>, AdbcError> {
    match config.driver {
        DatabaseDriver::ClickHouse => clickhouse::execute_query(config, query).await,
        ref other => Err(AdbcError::Unsupported(format!(
            "direct query execution not yet supported for {:?}; use the Python SDK with the native connector",
            other
        ))),
    }
}

async fn dispatch_tile(
    config: &ConnectionConfig,
    query: &str,
    row_offset: u64,
    row_limit: u64,
) -> Result<Vec<RecordBatch>, AdbcError> {
    match config.driver {
        DatabaseDriver::ClickHouse => {
            clickhouse::execute_tile(config, query, row_offset, row_limit).await
        }
        ref other => Err(AdbcError::Unsupported(format!(
            "tile execution not yet supported for {:?}",
            other
        ))),
    }
}

async fn dispatch_count(config: &ConnectionConfig, query: &str) -> Result<u64, AdbcError> {
    match config.driver {
        DatabaseDriver::ClickHouse => clickhouse::count_rows(config, query).await,
        ref other => Err(AdbcError::Unsupported(format!(
            "row counting not yet supported for {:?}",
            other
        ))),
    }
}

async fn dispatch_schema(config: &ConnectionConfig, query: &str) -> Result<SchemaRef, AdbcError> {
    match config.driver {
        DatabaseDriver::ClickHouse => clickhouse::infer_schema(config, query).await,
        ref other => Err(AdbcError::Unsupported(format!(
            "schema inference not yet supported for {:?}",
            other
        ))),
    }
}
