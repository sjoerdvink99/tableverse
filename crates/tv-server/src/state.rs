use std::sync::Arc;
use tv_engine::job_registry::JobRegistry;
use tv_engine::Engine;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
    pub cache: Arc<crate::cache::OptionalCache>,
    pub tile_cache_ttl: u64,
    pub job_registry: Arc<JobRegistry>,
}

impl AppState {
    pub fn new(engine: Engine, redis_url: Option<String>) -> Self {
        let job_registry = engine.job_registry();
        let tile_cache_ttl = std::env::var("TILE_CACHE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600u64);
        let cache = Arc::new(crate::cache::OptionalCache::new(redis_url, 0));
        cache
            .clone()
            .spawn_cleanup(std::time::Duration::from_secs(60));
        Self {
            engine: Arc::new(engine),
            cache,
            tile_cache_ttl,
            job_registry,
        }
    }
}
