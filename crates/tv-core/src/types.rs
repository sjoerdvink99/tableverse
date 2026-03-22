use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceFormat {
    Parquet,
    Csv,
    Arrow,
    Json,
    Delta,
    Iceberg,
    Database,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    LocalFile,
    S3,
    Gcs,
    AzureBlob,
    Http,
    Delta,
    Iceberg,
    Postgres,
    Mysql,
    HuggingFace,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Credentials {
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub session_token: Option<String>,
    pub endpoint: Option<String>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub index: usize,
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CardinalityCategory {
    Constant,
    Binary,
    LowCardinality,
    Categorical,
    HighCardinality,
    Unique,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quantiles {
    pub p1: f64,
    pub p5: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
    pub p99: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopValue {
    pub value: serde_json::Value,
    pub count: u64,
    pub rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickColumnStats {
    pub index: usize,
    pub null_count: u64,
    pub null_rate: f64,
    pub min: Option<serde_json::Value>,
    pub max: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRecommendation {
    pub kind: String,
    pub message: String,
}

fn default_tile_rows() -> u32 {
    256
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMeta {
    pub id: String,
    pub name: String,
    pub uri: String,
    #[serde(default)]
    pub files: Vec<String>,
    pub format: SourceFormat,
    pub kind: SourceKind,
    pub n_rows: u64,
    pub n_cols: usize,
    pub columns: Vec<ColumnInfo>,
    #[serde(default)]
    pub quick_stats: Vec<QuickColumnStats>,
    #[serde(default = "default_tile_rows")]
    pub tile_rows: u32,
    #[serde(default)]
    pub file_mtime_secs: u64,
    #[serde(default)]
    pub file_size_bytes: u64,
    #[serde(default)]
    pub recommendations: Vec<SourceRecommendation>,
    #[serde(default)]
    pub pre_sorted_by: Option<Vec<crate::expr::SortKey>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileRequest {
    pub source_id: String,
    pub row: u64,
    pub col: usize,
    pub rows: u64,
    pub cols: usize,
    pub sort: Option<SortSpec>,
    pub filter: Option<FilterExpr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileResponse {
    pub source_id: String,
    pub row: u64,
    pub col: usize,
    pub data: Vec<u8>,
    #[serde(default)]
    pub is_provisional: bool,
    #[serde(default)]
    pub job_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchTileRequest {
    pub row: u64,
    pub col: usize,
    pub rows: u64,
    pub cols: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortSpec {
    pub column: String,
    pub descending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FilterExpr {
    Eq {
        column: String,
        value: serde_json::Value,
    },
    Ne {
        column: String,
        value: serde_json::Value,
    },
    Gt {
        column: String,
        value: serde_json::Value,
    },
    Gte {
        column: String,
        value: serde_json::Value,
    },
    Lt {
        column: String,
        value: serde_json::Value,
    },
    Lte {
        column: String,
        value: serde_json::Value,
    },
    Contains {
        column: String,
        value: String,
    },
    IsNull {
        column: String,
    },
    IsNotNull {
        column: String,
    },
    And {
        exprs: Vec<FilterExpr>,
    },
    Or {
        exprs: Vec<FilterExpr>,
    },
    Not {
        expr: Box<FilterExpr>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnStats {
    pub column: String,
    pub index: usize,
    pub data_type: String,
    pub count: u64,
    pub null_count: u64,
    pub null_rate: f64,
    pub distinct_count: Option<u64>,
    pub min: Option<serde_json::Value>,
    pub max: Option<serde_json::Value>,
    pub mean: Option<f64>,
    pub quantiles: Option<Quantiles>,
    pub histogram: Option<Vec<HistogramBucket>>,
    pub top_values: Option<Vec<TopValue>>,
    pub cardinality_category: CardinalityCategory,
}

#[derive(Debug, Clone)]
pub struct QuantileSketch {
    pub sorted_values: Vec<f64>,
}

impl QuantileSketch {
    pub fn new(mut values: Vec<f64>) -> Self {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Self {
            sorted_values: values,
        }
    }

    pub fn cdf(&self, x: f64) -> f64 {
        if self.sorted_values.is_empty() {
            return 0.5;
        }
        let pos = self.sorted_values.partition_point(|&v| v <= x);
        pos as f64 / self.sorted_values.len() as f64
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramBucket {
    pub lo: f64,
    pub hi: f64,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationMatrix {
    pub columns: Vec<String>,
    pub matrix: Vec<Vec<Option<f64>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    pub rows: Vec<u64>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterSourceRequest {
    pub uri: String,
    pub name: Option<String>,
    pub profile: Option<String>,
    pub credentials: Option<Credentials>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_meta_serde_roundtrip() {
        let meta = SourceMeta {
            id: "id1".into(),
            name: "test".into(),
            uri: "/tmp/test.parquet".into(),
            files: vec![],
            format: SourceFormat::Parquet,
            kind: SourceKind::LocalFile,
            n_rows: 100,
            n_cols: 3,
            columns: vec![ColumnInfo {
                index: 0,
                name: "x".into(),
                data_type: "Int64".into(),
                nullable: false,
            }],
            quick_stats: vec![],
            tile_rows: 256,
            file_mtime_secs: 0,
            file_size_bytes: 0,
            recommendations: vec![],
            pre_sorted_by: None,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: SourceMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, meta.id);
        assert_eq!(back.n_rows, meta.n_rows);
        assert_eq!(back.columns.len(), 1);
    }

    #[test]
    fn column_stats_default_values() {
        let stats = ColumnStats {
            column: "x".into(),
            index: 0,
            data_type: "Float64".into(),
            count: 0,
            null_count: 0,
            null_rate: 0.0,
            distinct_count: None,
            min: None,
            max: None,
            mean: None,
            quantiles: None,
            histogram: None,
            top_values: None,
            cardinality_category: CardinalityCategory::Unknown,
        };
        assert_eq!(stats.count, 0);
        assert!(stats.distinct_count.is_none());
    }

    #[test]
    fn tile_request_fields() {
        let req = TileRequest {
            source_id: "src1".into(),
            row: 256,
            col: 64,
            rows: 256,
            cols: 64,
            sort: None,
            filter: None,
        };
        assert_eq!(req.row, 256);
        assert_eq!(req.col, 64);
    }

    #[test]
    fn column_info_fields() {
        let ci = ColumnInfo {
            index: 2,
            name: "salary".into(),
            data_type: "Float64".into(),
            nullable: true,
        };
        assert_eq!(ci.index, 2);
        assert_eq!(ci.name, "salary");
        assert!(ci.nullable);
    }
}
