use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

struct Entry {
    data: Arc<Vec<u8>>,
    bytes: usize,
    freq: std::sync::atomic::AtomicU32,
    expires_at: Option<Instant>,
}

pub struct OptionalCache {
    inner: DashMap<String, Entry>,
    current_bytes: AtomicUsize,
    max_bytes: usize,
}

impl OptionalCache {
    pub fn new(redis_url: Option<String>, _max_size: usize) -> Self {
        if let Some(url) = redis_url {
            tracing::info!(url = %url, "redis caching not implemented, using in-process cache");
        }
        let max_bytes = std::env::var("TILE_CACHE_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2usize * 1024 * 1024 * 1024);
        Self {
            inner: DashMap::new(),
            current_bytes: AtomicUsize::new(0),
            max_bytes,
        }
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let entry = self.inner.get(key)?;
        if let Some(exp) = entry.expires_at {
            if Instant::now() > exp {
                drop(entry);
                self.inner.remove(key);
                return None;
            }
        }
        entry.freq.fetch_add(1, Ordering::Relaxed);
        Some((*entry.data).clone())
    }

    pub fn set(&self, key: &str, value: Vec<u8>, ttl_secs: u64) {
        let bytes = value.len();
        let expires_at = if ttl_secs > 0 {
            Some(Instant::now() + Duration::from_secs(ttl_secs))
        } else {
            None
        };

        if let Some(old) = self.inner.remove(key) {
            self.current_bytes.fetch_sub(old.1.bytes, Ordering::Relaxed);
        }

        self.evict_if_needed(bytes);

        self.inner.insert(
            key.to_string(),
            Entry {
                data: Arc::new(value),
                bytes,
                freq: std::sync::atomic::AtomicU32::new(1),
                expires_at,
            },
        );
        self.current_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn byte_size(&self) -> u64 {
        self.current_bytes.load(Ordering::Relaxed) as u64
    }

    pub fn entry_count(&self) -> u64 {
        self.inner.len() as u64
    }

    pub fn invalidate_prefix(&self, prefix: &str) {
        let keys: Vec<String> = self
            .inner
            .iter()
            .filter(|e| e.key().starts_with(prefix))
            .map(|e| e.key().clone())
            .collect();
        for k in keys {
            if let Some(removed) = self.inner.remove(&k) {
                self.current_bytes
                    .fetch_sub(removed.1.bytes, Ordering::Relaxed);
            }
        }
    }

    pub fn spawn_cleanup(self: Arc<Self>, interval: Duration) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let now = Instant::now();
                let expired: Vec<String> = self
                    .inner
                    .iter()
                    .filter(|e| e.expires_at.map(|exp| now > exp).unwrap_or(false))
                    .map(|e| e.key().clone())
                    .collect();
                for k in expired {
                    if let Some(removed) = self.inner.remove(&k) {
                        self.current_bytes
                            .fetch_sub(removed.1.bytes, Ordering::Relaxed);
                    }
                }
            }
        });
    }

    fn evict_if_needed(&self, needed: usize) {
        let current = self.current_bytes.load(Ordering::Relaxed);
        if current + needed <= self.max_bytes {
            return;
        }
        let mut entries: Vec<(String, u32)> = self
            .inner
            .iter()
            .map(|e| (e.key().clone(), e.freq.load(Ordering::Relaxed)))
            .collect();
        entries.sort_by_key(|(_, freq)| *freq);

        for (key, _) in entries {
            if self.current_bytes.load(Ordering::Relaxed) + needed <= self.max_bytes {
                break;
            }
            if let Some(removed) = self.inner.remove(&key) {
                self.current_bytes
                    .fetch_sub(removed.1.bytes, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    fn make_cache() -> OptionalCache {
        OptionalCache::new(None, 1024)
    }

    #[test]
    fn cache_set_get() {
        let c = make_cache();
        c.set("k1", vec![1, 2, 3], 0);
        assert_eq!(c.get("k1"), Some(vec![1, 2, 3]));
    }

    #[test]
    fn cache_get_miss() {
        let c = make_cache();
        assert_eq!(c.get("missing"), None);
    }

    #[test]
    fn cache_ttl_expiry() {
        let c = make_cache();
        c.set("k", vec![42], 1);
        assert!(c.get("k").is_some());
        sleep(Duration::from_millis(1100));
        assert!(c.get("k").is_none());
    }

    #[test]
    fn cache_eviction_byte_budget() {
        let mut c = OptionalCache::new(None, 1024);
        c.max_bytes = 10;
        c.set("a", vec![1, 2, 3, 4, 5], 0);
        c.set("b", vec![6, 7, 8, 9, 10], 0);
        c.set("c", vec![11, 12, 13, 14, 15], 0);
        assert!(c.get("c").is_some());
    }

    #[test]
    fn cache_invalidate_prefix() {
        let c = make_cache();
        c.set("src1:tile:1", vec![1], 0);
        c.set("src1:tile:2", vec![2], 0);
        c.set("src2:tile:1", vec![3], 0);
        c.invalidate_prefix("src1:");
        assert!(c.get("src1:tile:1").is_none());
        assert!(c.get("src1:tile:2").is_none());
        assert!(c.get("src2:tile:1").is_some());
    }

    #[test]
    fn cache_overwrite_key() {
        let c = make_cache();
        c.set("k", vec![1], 0);
        c.set("k", vec![2], 0);
        assert_eq!(c.get("k"), Some(vec![2]));
    }
}
