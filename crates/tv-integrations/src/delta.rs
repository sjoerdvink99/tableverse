use crate::{IntegrationError, ResolvedFormat, ResolvedSource};
use std::collections::HashSet;
use tv_core::Credentials;

pub async fn resolve(
    uri: &str,
    _credentials: Option<&Credentials>,
) -> Result<ResolvedSource, IntegrationError> {
    let table_path = strip_scheme(uri);

    if table_path.starts_with('/') || (!table_path.contains("://")) {
        resolve_local(table_path).await
    } else {
        Err(IntegrationError::Resolution(
            "remote Delta tables require delta_rs crate; use local paths or S3 via register_source"
                .to_string(),
        ))
    }
}

async fn resolve_local(table_path: &str) -> Result<ResolvedSource, IntegrationError> {
    let log_dir = std::path::Path::new(table_path).join("_delta_log");
    if !log_dir.exists() {
        return Err(IntegrationError::NotFound(format!(
            "_delta_log not found at {table_path}"
        )));
    }

    let mut add_files: HashSet<String> = HashSet::new();
    let mut remove_files: HashSet<String> = HashSet::new();
    let mut last_snapshot: Option<String> = None;

    let mut entries: Vec<_> = std::fs::read_dir(&log_dir)
        .map_err(|e| IntegrationError::Resolution(e.to_string()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .collect();

    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let path = entry.path();
        let content = std::fs::read_to_string(&path)
            .map_err(|e| IntegrationError::Resolution(e.to_string()))?;

        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            last_snapshot = Some(stem.to_string());
        }

        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(line)?;
            if let Some(add) = v.get("add") {
                if let Some(path_str) = add.get("path").and_then(|p| p.as_str()) {
                    let full = format!("{}/{}", table_path.trim_end_matches('/'), path_str);
                    remove_files.remove(&full);
                    add_files.insert(full);
                }
            }
            if let Some(remove) = v.get("remove") {
                if let Some(path_str) = remove.get("path").and_then(|p| p.as_str()) {
                    let full = format!("{}/{}", table_path.trim_end_matches('/'), path_str);
                    add_files.remove(&full);
                    remove_files.insert(full);
                }
            }
        }
    }

    let mut parquet_uris: Vec<String> = add_files.into_iter().collect();
    parquet_uris.sort();

    if parquet_uris.is_empty() {
        return Err(IntegrationError::NotFound(
            "no active Parquet files in Delta table".to_string(),
        ));
    }

    Ok(ResolvedSource {
        parquet_uris,
        snapshot_id: last_snapshot,
        source_format: ResolvedFormat::DeltaParquet,
    })
}

fn strip_scheme(uri: &str) -> &str {
    if let Some(rest) = uri.strip_prefix("delta://") {
        rest
    } else if let Some(rest) = uri.strip_prefix("delta+s3://") {
        rest
    } else {
        uri
    }
}
