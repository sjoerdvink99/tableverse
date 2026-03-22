use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json;
use tracing::warn;
use tv_core::SourceMeta;

pub struct CatalogStore {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl CatalogStore {
    pub fn open(path: &Path) -> Result<Self, CatalogStoreError> {
        let conn = rusqlite::Connection::open(path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.initialize()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, CatalogStoreError> {
        let conn = rusqlite::Connection::open_in_memory()?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> Result<(), CatalogStoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sources (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                uri TEXT NOT NULL,
                meta_json TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );
            CREATE INDEX IF NOT EXISTS idx_sources_created ON sources(created_at DESC);",
        )?;
        Ok(())
    }

    pub fn upsert(&self, meta: &SourceMeta) -> Result<(), CatalogStoreError> {
        let json = serde_json::to_string(meta)
            .map_err(|e| CatalogStoreError::Serialization(e.to_string()))?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sources (id, name, uri, meta_json)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, uri=excluded.uri, meta_json=excluded.meta_json",
            rusqlite::params![meta.id, meta.name, meta.uri, json],
        )?;
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<(), CatalogStoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sources WHERE id = ?1", rusqlite::params![id])?;
        Ok(())
    }

    pub fn load_all(&self) -> Result<Vec<SourceMeta>, CatalogStoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT meta_json FROM sources ORDER BY created_at DESC")?;
        let metas = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| match r {
                Ok(json) => match serde_json::from_str::<SourceMeta>(&json) {
                    Ok(m) => Some(m),
                    Err(e) => {
                        warn!(error = %e, "skipping malformed source row");
                        None
                    }
                },
                Err(e) => {
                    warn!(error = %e, "row fetch error");
                    None
                }
            })
            .collect();
        Ok(metas)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogStoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serialization(String),
}
