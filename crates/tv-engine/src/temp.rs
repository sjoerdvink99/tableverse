use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::error::EngineError;

static INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TempRoot {
    root: PathBuf,
}

impl TempRoot {
    pub fn new() -> Result<Arc<Self>, EngineError> {
        let root = if let Ok(dir) = std::env::var("TABLEVERSE_TEMP_DIR") {
            PathBuf::from(dir)
        } else {
            let idx = INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            std::env::temp_dir().join(format!("tableverse_tv_{pid}_{idx}"))
        };

        std::fs::create_dir_all(&root)?;

        Ok(Arc::new(Self { root }))
    }

    pub fn view_dir(&self, source_id: &str, view_hash: &str) -> Result<TempDirGuard, EngineError> {
        let path = self.root.join(source_id).join(view_hash);
        std::fs::create_dir_all(&path)?;
        Ok(TempDirGuard { path })
    }

    pub fn cleanup_source(&self, source_id: &str) {
        let path = self.root.join(source_id);
        let _ = std::fs::remove_dir_all(&path);
    }

    pub fn uploads_dir(&self) -> Result<PathBuf, EngineError> {
        let path = self.root.join("uploads");
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    pub fn upload_path(&self, id: &str, extension: &str) -> Result<PathBuf, EngineError> {
        let dir = self.uploads_dir()?;
        Ok(dir.join(format!("{}.{}", id, extension)))
    }

    pub fn cleanup_upload(&self, id: &str) {
        let dir = self.root.join("uploads");
        for ext in &["arrow", "parquet"] {
            let path = dir.join(format!("{}.{}", id, ext));
            let _ = std::fs::remove_file(&path);
        }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub struct TempDirGuard {
    pub path: PathBuf,
}

impl TempDirGuard {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
