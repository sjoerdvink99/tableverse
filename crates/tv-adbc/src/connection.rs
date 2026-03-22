use crate::error::AdbcError;

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub driver: DatabaseDriver,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub extra_params: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DatabaseDriver {
    ClickHouse,
    PostgreSql,
    MySql,
    Snowflake,
    BigQuery,
    DuckDb,
    Generic(String),
}

impl ConnectionConfig {
    pub fn from_uri(uri: &str) -> Result<Self, AdbcError> {
        let url =
            url::Url::parse(uri).map_err(|e| AdbcError::InvalidConnectionString(e.to_string()))?;

        let driver = match url.scheme() {
            "clickhouse" | "clickhouse+http" | "clickhouse+https" => DatabaseDriver::ClickHouse,
            "postgresql" | "postgres" => DatabaseDriver::PostgreSql,
            "mysql" => DatabaseDriver::MySql,
            "snowflake" => DatabaseDriver::Snowflake,
            "bigquery" => DatabaseDriver::BigQuery,
            "duckdb" => DatabaseDriver::DuckDb,
            other => DatabaseDriver::Generic(other.to_string()),
        };

        let host = url.host_str().unwrap_or("localhost").to_string();
        let port = url.port().unwrap_or_else(|| default_port(&driver));
        let database = url.path().trim_start_matches('/').to_string();
        let username = if url.username().is_empty() {
            None
        } else {
            Some(url.username().to_string())
        };
        let password = url.password().map(|s| s.to_string());

        let mut extra_params = std::collections::HashMap::new();
        for (k, v) in url.query_pairs() {
            extra_params.insert(k.to_string(), v.to_string());
        }

        Ok(Self {
            driver,
            host,
            port,
            database,
            username,
            password,
            extra_params,
        })
    }
}

fn default_port(driver: &DatabaseDriver) -> u16 {
    match driver {
        DatabaseDriver::ClickHouse => 8123,
        DatabaseDriver::PostgreSql => 5432,
        DatabaseDriver::MySql => 3306,
        DatabaseDriver::Snowflake => 443,
        DatabaseDriver::BigQuery => 443,
        DatabaseDriver::DuckDb => 0,
        DatabaseDriver::Generic(_) => 5432,
    }
}
