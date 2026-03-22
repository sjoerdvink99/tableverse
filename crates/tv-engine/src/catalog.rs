use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tv_core::{SourceFormat, SourceKind, SourceMeta};

pub struct Catalog {
    sources: Arc<RwLock<HashMap<String, SourceMeta>>>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new()
    }
}

impl Catalog {
    pub fn new() -> Self {
        Self {
            sources: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn insert(&self, meta: SourceMeta) {
        if let Ok(mut map) = self.sources.write() {
            map.insert(meta.id.clone(), meta);
        }
    }

    pub fn get(&self, id: &str) -> Option<SourceMeta> {
        self.sources.read().ok()?.get(id).cloned()
    }

    pub fn remove(&self, id: &str) -> bool {
        self.sources
            .write()
            .ok()
            .map(|mut m| m.remove(id).is_some())
            .unwrap_or(false)
    }

    pub fn list(&self) -> Vec<SourceMeta> {
        self.sources
            .read()
            .ok()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }
}

pub fn view_name(id: &str) -> String {
    format!("source_{}", id.replace('-', "_"))
}

pub fn detect_kind(uri: &str) -> SourceKind {
    let lower = uri.to_lowercase();
    if lower.starts_with("s3://") || lower.starts_with("s3a://") {
        SourceKind::S3
    } else if lower.starts_with("gs://") || lower.starts_with("gcs://") {
        SourceKind::Gcs
    } else if lower.starts_with("az://") || lower.starts_with("abfs://") {
        SourceKind::AzureBlob
    } else if lower.starts_with("hf://") || lower.starts_with("huggingface://") {
        SourceKind::HuggingFace
    } else if lower.starts_with("delta://") || lower.starts_with("delta+s3://") {
        SourceKind::Delta
    } else if lower.starts_with("iceberg://") {
        SourceKind::Iceberg
    } else if lower.starts_with("postgres://") || lower.starts_with("postgresql://") {
        SourceKind::Postgres
    } else if lower.starts_with("mysql://") {
        SourceKind::Mysql
    } else if lower.starts_with("http://") || lower.starts_with("https://") {
        SourceKind::Http
    } else {
        SourceKind::LocalFile
    }
}

pub fn detect_format(uri: &str, kind: &SourceKind) -> SourceFormat {
    match kind {
        SourceKind::Delta => return SourceFormat::Delta,
        SourceKind::Iceberg => return SourceFormat::Iceberg,
        SourceKind::Postgres | SourceKind::Mysql => return SourceFormat::Database,
        _ => {}
    }

    let lower = uri.to_lowercase();
    let path_part = lower.split('?').next().unwrap_or(&lower);

    if path_part.ends_with(".parquet") || path_part.contains(".parquet/") {
        SourceFormat::Parquet
    } else if path_part.ends_with(".csv") || path_part.ends_with(".tsv") {
        SourceFormat::Csv
    } else if path_part.ends_with(".arrow")
        || path_part.ends_with(".feather")
        || path_part.ends_with(".ipc")
    {
        SourceFormat::Arrow
    } else if path_part.ends_with(".json")
        || path_part.ends_with(".jsonl")
        || path_part.ends_with(".ndjson")
    {
        SourceFormat::Json
    } else {
        SourceFormat::Parquet
    }
}

pub fn infer_name(uri: &str) -> String {
    uri.split('/')
        .next_back()
        .unwrap_or(uri)
        .split('.')
        .next()
        .unwrap_or(uri)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tv_core::{SourceFormat, SourceKind, SourceMeta};

    fn make_meta(id: &str, uri: &str, kind: SourceKind, format: SourceFormat) -> SourceMeta {
        SourceMeta {
            id: id.into(),
            name: infer_name(uri),
            uri: uri.into(),
            files: vec![],
            format,
            kind,
            n_rows: 0,
            n_cols: 0,
            columns: vec![],
            quick_stats: vec![],
            tile_rows: 256,
            file_mtime_secs: 0,
            file_size_bytes: 0,
            recommendations: vec![],
            pre_sorted_by: None,
        }
    }

    #[test]
    fn catalog_insert_get_remove() {
        let cat = Catalog::new();
        let meta = make_meta(
            "id1",
            "/tmp/test.parquet",
            SourceKind::LocalFile,
            SourceFormat::Parquet,
        );
        cat.insert(meta.clone());
        assert!(cat.get("id1").is_some());
        assert!(cat.remove("id1"));
        assert!(cat.get("id1").is_none());
    }

    #[test]
    fn catalog_list_returns_all() {
        let cat = Catalog::new();
        cat.insert(make_meta(
            "a",
            "/tmp/a.parquet",
            SourceKind::LocalFile,
            SourceFormat::Parquet,
        ));
        cat.insert(make_meta(
            "b",
            "/tmp/b.parquet",
            SourceKind::LocalFile,
            SourceFormat::Parquet,
        ));
        assert_eq!(cat.list().len(), 2);
    }

    #[test]
    fn catalog_remove_nonexistent() {
        let cat = Catalog::new();
        assert!(!cat.remove("does_not_exist"));
    }

    #[test]
    fn detect_kind_local() {
        assert_eq!(detect_kind("/tmp/file.parquet"), SourceKind::LocalFile);
    }

    #[test]
    fn detect_kind_s3() {
        assert_eq!(detect_kind("s3://bucket/file.parquet"), SourceKind::S3);
    }

    #[test]
    fn detect_kind_gcs() {
        assert_eq!(detect_kind("gs://bucket/file.parquet"), SourceKind::Gcs);
    }

    #[test]
    fn detect_format_parquet() {
        assert!(matches!(
            detect_format("/tmp/data.parquet", &SourceKind::LocalFile),
            SourceFormat::Parquet
        ));
    }

    #[test]
    fn detect_format_csv() {
        assert!(matches!(
            detect_format("/tmp/data.csv", &SourceKind::LocalFile),
            SourceFormat::Csv
        ));
    }

    #[test]
    fn detect_format_json() {
        assert!(matches!(
            detect_format("/tmp/data.json", &SourceKind::LocalFile),
            SourceFormat::Json
        ));
    }

    #[test]
    fn infer_name_from_path() {
        assert_eq!(infer_name("/tmp/my_data.parquet"), "my_data");
    }
}
