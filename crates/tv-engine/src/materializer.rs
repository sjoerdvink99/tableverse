use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use tv_core::SortKey;

use crate::error::EngineError;
use crate::spill::SpilledRun;
use crate::temp::TempDirGuard;

use std::path::PathBuf;

pub enum MaterializedView {
    Batches {
        batches: Vec<RecordBatch>,
        total_rows: u64,
    },
    RowCount {
        count: u64,
    },
    SortedRuns {
        runs: Vec<SpilledRun>,
        cumulative_rows: Vec<u64>,
        schema: SchemaRef,
        sort_keys: Vec<SortKey>,
        total_rows: u64,
        dedup_columns: Option<Vec<String>>,
        _guard: Arc<TempDirGuard>,
    },
    AggregateResult {
        run: SpilledRun,
        schema: SchemaRef,
        total_rows: u64,
        _guard: Arc<TempDirGuard>,
    },
    SortIndexBacked {
        index_path: PathBuf,
        source_path: String,
        n_cols: usize,
        total_rows: u64,
    },
    SparseSortIndexBacked {
        index: Arc<crate::sparse_sort_index::SparseSortIndex>,
        spill_path: PathBuf,
        schema: SchemaRef,
        total_rows: u64,
        _guard: Arc<TempDirGuard>,
    },
    ProvisionalAgg {
        batches: Vec<RecordBatch>,
        total_rows: u64,
        job_id: String,
    },
    ProvisionalSort {
        runs: Vec<SpilledRun>,
        cumulative_rows: Vec<u64>,
        schema: SchemaRef,
        sort_keys: Vec<SortKey>,
        total_rows: u64,
        estimated_total_rows: u64,
        job_id: String,
        _guard: Arc<TempDirGuard>,
    },
    BitmapGroupBy {
        batches: Vec<RecordBatch>,
        total_rows: u64,
    },
}

impl MaterializedView {
    pub fn row_count(&self) -> u64 {
        match self {
            MaterializedView::Batches { total_rows, .. } => *total_rows,
            MaterializedView::RowCount { count } => *count,
            MaterializedView::SortedRuns { total_rows, .. } => *total_rows,
            MaterializedView::AggregateResult { total_rows, .. } => *total_rows,
            MaterializedView::SortIndexBacked { total_rows, .. } => *total_rows,
            MaterializedView::SparseSortIndexBacked { total_rows, .. } => *total_rows,
            MaterializedView::ProvisionalAgg { total_rows, .. } => *total_rows,
            MaterializedView::ProvisionalSort { total_rows, .. } => *total_rows,
            MaterializedView::BitmapGroupBy { total_rows, .. } => *total_rows,
        }
    }

    pub fn job_id(&self) -> Option<&str> {
        match self {
            MaterializedView::ProvisionalAgg { job_id, .. } => Some(job_id.as_str()),
            MaterializedView::ProvisionalSort { job_id, .. } => Some(job_id.as_str()),
            _ => None,
        }
    }

    pub fn is_provisional(&self) -> bool {
        matches!(
            self,
            MaterializedView::ProvisionalAgg { .. } | MaterializedView::ProvisionalSort { .. }
        )
    }

    fn byte_size(&self) -> u64 {
        match self {
            MaterializedView::Batches { batches, .. }
            | MaterializedView::ProvisionalAgg { batches, .. }
            | MaterializedView::BitmapGroupBy { batches, .. } => batches
                .iter()
                .flat_map(|b| (0..b.num_columns()).map(|i| b.column(i).get_array_memory_size()))
                .sum::<usize>()
                as u64,
            MaterializedView::RowCount { .. } => 16,
            MaterializedView::SortedRuns { runs, .. } => runs.iter().map(|r| r.file_size()).sum(),
            MaterializedView::ProvisionalSort { runs, .. } => {
                runs.iter().map(|r| r.file_size()).sum()
            }
            MaterializedView::AggregateResult { run, .. } => run.file_size(),
            MaterializedView::SortIndexBacked { index_path, .. } => {
                std::fs::metadata(index_path).map(|m| m.len()).unwrap_or(0)
            }
            MaterializedView::SparseSortIndexBacked {
                index, spill_path, ..
            } => index.byte_size() + std::fs::metadata(spill_path).map(|m| m.len()).unwrap_or(0),
        }
    }

    fn is_disk_backed(&self) -> bool {
        matches!(
            self,
            MaterializedView::SortedRuns { .. }
                | MaterializedView::AggregateResult { .. }
                | MaterializedView::SparseSortIndexBacked { .. }
                | MaterializedView::SortIndexBacked { .. }
                | MaterializedView::ProvisionalSort { .. }
        )
    }
}

struct CacheEntry {
    view: Arc<MaterializedView>,
    bytes: u64,
    seq: u64,
    is_disk_backed: bool,
}

type PendingVal = Option<Result<Arc<MaterializedView>, String>>;

pub struct ViewMaterializer {
    entries: RwLock<HashMap<String, CacheEntry>>,
    pending: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::watch::Sender<PendingVal>>>>,
    current_bytes: AtomicU64,
    disk_backed_count: AtomicU64,
    access_seq: AtomicU64,
    max_memory_bytes: u64,
    max_disk_backed_count: u64,
}

impl Default for ViewMaterializer {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewMaterializer {
    pub fn new() -> Self {
        let max_memory_bytes = std::env::var("VIEW_CACHE_MAX_BYTES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2u64 * 1024 * 1024 * 1024);
        let max_disk_backed_count = std::env::var("VIEW_CACHE_MAX_DISK_ENTRIES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(32u64);
        Self {
            entries: RwLock::new(HashMap::new()),
            pending: tokio::sync::Mutex::new(HashMap::new()),
            current_bytes: AtomicU64::new(0),
            disk_backed_count: AtomicU64::new(0),
            access_seq: AtomicU64::new(0),
            max_memory_bytes,
            max_disk_backed_count,
        }
    }

    pub async fn get(&self, key: &str) -> Option<Arc<MaterializedView>> {
        let mut entries = self.entries.write().await;
        let entry = entries.get_mut(key)?;
        entry.seq = self.access_seq.fetch_add(1, Ordering::Relaxed);
        Some(entry.view.clone())
    }

    pub async fn get_or_materialize<F, Fut>(
        &self,
        key: String,
        builder: F,
    ) -> Result<Arc<MaterializedView>, EngineError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<MaterializedView, EngineError>>,
    {
        if let Some(view) = self.get(&key).await {
            debug!(key = %key, rows = view.row_count(), "materializer cache hit");
            return Ok(view);
        }

        let maybe_rx = {
            let mut pending = self.pending.lock().await;
            if let Some(tx) = pending.get(&key) {
                Some(tx.subscribe())
            } else {
                let (tx, _) = tokio::sync::watch::channel::<PendingVal>(None);
                let tx = Arc::new(tx);
                pending.insert(key.clone(), tx);
                None
            }
        };

        if let Some(mut rx) = maybe_rx {
            debug!(key = %key, "waiting for concurrent materialisation");
            rx.wait_for(|v| v.is_some())
                .await
                .map_err(|_| EngineError::Query("concurrent builder was cancelled".into()))?;
            let result = (*rx.borrow()).clone().unwrap();
            return result.map_err(EngineError::Query);
        }

        info!(key = %key, "materialising view");
        let t0 = std::time::Instant::now();
        let result = builder().await;

        let tx = self.pending.lock().await.remove(&key);

        match result {
            Ok(view) => {
                let elapsed_ms = t0.elapsed().as_millis();
                info!(key = %key, rows = view.row_count(), elapsed_ms = elapsed_ms, "materialisation complete");
                let is_disk_backed = view.is_disk_backed();
                let bytes = if is_disk_backed { 0 } else { view.byte_size() };
                self.evict_if_needed(is_disk_backed).await;
                let view = Arc::new(view);
                let seq = self.access_seq.fetch_add(1, Ordering::Relaxed);
                let mut entries = self.entries.write().await;
                if let Some(existing) = entries.get_mut(&key) {
                    existing.seq = seq;
                    let existing_view = existing.view.clone();
                    drop(entries);
                    if let Some(tx) = tx {
                        let _ = tx.send(Some(Ok(existing_view.clone())));
                    }
                    return Ok(existing_view);
                }
                entries.insert(
                    key,
                    CacheEntry {
                        view: view.clone(),
                        bytes,
                        seq,
                        is_disk_backed,
                    },
                );
                drop(entries);
                if is_disk_backed {
                    self.disk_backed_count.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.current_bytes.fetch_add(bytes, Ordering::Relaxed);
                }
                if let Some(tx) = tx {
                    let _ = tx.send(Some(Ok(view.clone())));
                }
                Ok(view)
            }
            Err(e) => {
                warn!(key = %key, error = %e, "materialisation failed");
                if let Some(tx) = tx {
                    let _ = tx.send(Some(Err(e.to_string())));
                }
                Err(e)
            }
        }
    }

    pub async fn invalidate_source(&self, source_id: &str) {
        let prefix = format!("{source_id}:");
        let mut entries = self.entries.write().await;
        let mut freed_bytes = 0u64;
        let mut freed_disk = 0u64;
        entries.retain(|k, v| {
            if k.starts_with(&prefix) {
                if v.is_disk_backed {
                    freed_disk += 1;
                } else {
                    freed_bytes += v.bytes;
                }
                false
            } else {
                true
            }
        });
        if freed_bytes > 0 {
            self.current_bytes.fetch_sub(freed_bytes, Ordering::Relaxed);
        }
        if freed_disk > 0 {
            self.disk_backed_count
                .fetch_sub(freed_disk, Ordering::Relaxed);
        }
    }

    async fn evict_if_needed(&self, is_disk_backed: bool) {
        let mut entries = self.entries.write().await;

        if is_disk_backed {
            let count = self.disk_backed_count.load(Ordering::Relaxed);
            if count < self.max_disk_backed_count {
                return;
            }
            let mut disk_entries: Vec<(String, u64)> = entries
                .iter()
                .filter(|(_, v)| v.is_disk_backed)
                .map(|(k, v)| (k.clone(), v.seq))
                .collect();
            disk_entries.sort_by_key(|(_, seq)| *seq);
            let to_evict = (count + 1).saturating_sub(self.max_disk_backed_count) as usize;
            for (key, _) in disk_entries.into_iter().take(to_evict) {
                if let Some(entry) = entries.remove(&key) {
                    if !entry.is_disk_backed {
                        self.current_bytes.fetch_sub(entry.bytes, Ordering::Relaxed);
                    }
                    self.disk_backed_count.fetch_sub(1, Ordering::Relaxed);
                }
            }
        } else {
            let current = self.current_bytes.load(Ordering::Relaxed);
            if current < self.max_memory_bytes {
                return;
            }
            let mut mem_entries: Vec<(String, u64)> = entries
                .iter()
                .filter(|(_, v)| !v.is_disk_backed)
                .map(|(k, v)| (k.clone(), v.seq))
                .collect();
            mem_entries.sort_by_key(|(_, seq)| *seq);

            let mut freed = 0u64;
            for (key, _) in mem_entries {
                if self
                    .current_bytes
                    .load(Ordering::Relaxed)
                    .saturating_sub(freed)
                    < self.max_memory_bytes
                {
                    break;
                }
                if let Some(entry) = entries.remove(&key) {
                    freed += entry.bytes;
                }
            }
            if freed > 0 {
                self.current_bytes.fetch_sub(freed, Ordering::Relaxed);
            }
        }
    }

    pub async fn insert(&self, key: String, view: Arc<MaterializedView>) {
        let is_disk_backed = view.is_disk_backed();
        let bytes = if is_disk_backed { 0 } else { view.byte_size() };
        self.evict_if_needed(is_disk_backed).await;
        let mut entries = self.entries.write().await;
        let seq = self.access_seq.fetch_add(1, Ordering::Relaxed);
        if let Some(old) = entries.insert(
            key,
            CacheEntry {
                view,
                bytes,
                seq,
                is_disk_backed,
            },
        ) {
            if old.is_disk_backed {
                self.disk_backed_count.fetch_sub(1, Ordering::Relaxed);
            } else {
                self.current_bytes.fetch_sub(old.bytes, Ordering::Relaxed);
            }
        }
        if is_disk_backed {
            self.disk_backed_count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.current_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    pub async fn replace(&self, key: String, view: MaterializedView) {
        self.insert(key, Arc::new(view)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::int_string_batch;

    #[tokio::test]
    async fn materializer_get_miss() {
        let m = ViewMaterializer::new();
        assert!(m.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn materializer_get_or_materialize_caches() {
        let m = ViewMaterializer::new();
        let batch = int_string_batch(10);

        let view = m
            .get_or_materialize("key1".into(), || async {
                Ok(MaterializedView::Batches {
                    batches: vec![batch.clone()],
                    total_rows: 10,
                })
            })
            .await
            .unwrap();
        assert!(matches!(view.as_ref(), MaterializedView::Batches { .. }));
        let cached = m.get("key1").await;
        assert!(cached.is_some());
    }

    #[tokio::test]
    async fn materializer_invalidate_source_clears_prefix() {
        let m = ViewMaterializer::new();
        let batch = int_string_batch(5);

        m.get_or_materialize("src1:tile:abc".into(), || async {
            Ok(MaterializedView::Batches {
                batches: vec![batch.clone()],
                total_rows: 5,
            })
        })
        .await
        .unwrap();

        m.get_or_materialize("src2:tile:abc".into(), || async {
            Ok(MaterializedView::Batches {
                batches: vec![batch.clone()],
                total_rows: 5,
            })
        })
        .await
        .unwrap();

        m.invalidate_source("src1").await;
        assert!(m.get("src1:tile:abc").await.is_none());
        assert!(m.get("src2:tile:abc").await.is_some());
    }

    #[tokio::test]
    async fn materializer_replace_updates() {
        let m = ViewMaterializer::new();
        let batch1 = int_string_batch(5);
        let batch2 = int_string_batch(10);

        m.get_or_materialize("key".into(), || async {
            Ok(MaterializedView::Batches {
                batches: vec![batch1.clone()],
                total_rows: 5,
            })
        })
        .await
        .unwrap();

        m.replace(
            "key".into(),
            MaterializedView::Batches {
                batches: vec![batch2.clone()],
                total_rows: 10,
            },
        )
        .await;

        if let Some(v) = m.get("key").await {
            if let MaterializedView::Batches { total_rows, .. } = v.as_ref() {
                assert_eq!(*total_rows, 10);
            }
        }
    }

    #[tokio::test]
    async fn materializer_row_count_variant() {
        let m = ViewMaterializer::new();

        let view = m
            .get_or_materialize("count_key".into(), || async {
                Ok(MaterializedView::RowCount { count: 42 })
            })
            .await
            .unwrap();
        if let MaterializedView::RowCount { count } = view.as_ref() {
            assert_eq!(*count, 42);
        } else {
            panic!("expected RowCount variant");
        }
    }
}
