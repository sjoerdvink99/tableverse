use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use tv_core::ViewExpr;

const JOB_TTL: Duration = Duration::from_secs(600);
const BROADCAST_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum JobEvent {
    TileReady {
        row: u64,
        col: usize,
        view_hash: String,
    },
    Progress {
        rows_processed: u64,
        total_rows: u64,
    },
    Complete {
        total_rows: u64,
        elapsed_ms: u64,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobPhase {
    Sampling,
    FullScan {
        rows_processed: u64,
        total_rows: u64,
    },
    Complete,
    Failed(String),
}

pub struct Job {
    pub id: String,
    pub view_expr: ViewExpr,
    pub source_id: String,
    pub phase: Arc<RwLock<JobPhase>>,
    pub tx: broadcast::Sender<JobEvent>,
    pub created_at: Instant,
    pub view_hash: String,
}

impl Job {
    fn new(id: String, view_expr: ViewExpr, view_hash: String) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let source_id = view_expr.source_id.clone();
        Self {
            id,
            view_expr,
            source_id,
            phase: Arc::new(RwLock::new(JobPhase::FullScan {
                rows_processed: 0,
                total_rows: 0,
            })),
            tx,
            created_at: Instant::now(),
            view_hash,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<JobEvent> {
        self.tx.subscribe()
    }

    pub async fn emit(&self, event: JobEvent) {
        let _ = self.tx.send(event);
    }

    pub async fn set_phase(&self, phase: JobPhase) {
        *self.phase.write().await = phase;
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > JOB_TTL
    }
}

pub struct JobRegistry {
    jobs: Arc<RwLock<HashMap<String, Arc<Job>>>>,
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRegistry {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create_job(&self, view_expr: ViewExpr, view_hash: String) -> Arc<Job> {
        let id = uuid::Uuid::new_v4().to_string();
        let job = Arc::new(Job::new(id.clone(), view_expr, view_hash));
        self.jobs.write().await.insert(id, Arc::clone(&job));
        job
    }

    pub async fn register_with_id(
        &self,
        id: String,
        view_expr: ViewExpr,
        view_hash: String,
    ) -> Arc<Job> {
        let job = Arc::new(Job::new(id.clone(), view_expr, view_hash));
        self.jobs.write().await.insert(id, Arc::clone(&job));
        job
    }

    pub async fn get_job(&self, id: &str) -> Option<Arc<Job>> {
        let jobs = self.jobs.read().await;
        jobs.get(id).cloned()
    }

    pub async fn gc(&self) {
        let mut jobs = self.jobs.write().await;
        jobs.retain(|_, job| !job.is_expired());
    }

    pub async fn remove_for_source(&self, source_id: &str) {
        let mut jobs = self.jobs.write().await;
        jobs.retain(|_, job| job.source_id != source_id);
    }

    pub async fn job_count(&self) -> usize {
        self.jobs.read().await.len()
    }
}
