//! Background job store for fire-and-forget agent tasks.
//!
//! Use case: "go research X" while the user keeps chatting. Without
//! this, every Telegram message blocks until the agent's ReAct loop
//! finishes — which can be 30–60s for research or planning tasks.
//!
//! Design notes:
//!   - Jobs live in an Arc<RwLock<HashMap>> keyed by job id. In-memory
//!     only (Phase 19 scope); a process restart wipes them. Persistence
//!     would mean a SQLite table + the runtime carrying enough context
//!     to resume mid-task, which is a much bigger change.
//!   - Job state transitions are linear:
//!       Queued → Running → (Done | Failed | Cancelled)
//!   - The bg_tool spawns the actual agent runtime via tokio::spawn —
//!     this module just stores state + provides await-by-id.
//!   - Cancellation is cooperative: we set status=Cancelled and let the
//!     spawned task observe it on its own polling cycle. Forcing kill
//!     mid-run would corrupt session state.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BgStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl BgStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BgJob {
    pub id: String,
    pub task: String,
    pub name: Option<String>,
    pub status: BgStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub iterations: usize,
}

impl BgJob {
    fn new(task: String, name: Option<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            task,
            name,
            status: BgStatus::Queued,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            result: None,
            error: None,
            session_id: None,
            iterations: 0,
        }
    }

    pub fn short_id(&self) -> &str {
        // First 8 chars of UUID — enough for human reference,
        // collisions are astronomically unlikely with O(N) jobs.
        let n = self.id.len().min(8);
        &self.id[..n]
    }

    pub fn elapsed_ms(&self) -> Option<i64> {
        match (self.started_at, self.finished_at.or(Some(Utc::now()))) {
            (Some(s), Some(e)) => Some((e - s).num_milliseconds()),
            _ => None,
        }
    }
}

/// In-memory job store. Cheap clone — each handle shares the same
/// underlying RwLock. Pass to bg_tool + to the worker tasks.
#[derive(Clone)]
pub struct BgStore {
    inner: Arc<BgStoreInner>,
}

struct BgStoreInner {
    jobs: RwLock<HashMap<String, BgJob>>,
    /// Per-job notify so `wait` can sleep without polling.
    notifiers: RwLock<HashMap<String, Arc<Notify>>>,
}

impl BgStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(BgStoreInner {
                jobs: RwLock::new(HashMap::new()),
                notifiers: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub async fn enqueue(&self, task: String, name: Option<String>) -> BgJob {
        let job = BgJob::new(task, name);
        self.inner.jobs.write().await.insert(job.id.clone(), job.clone());
        self.inner
            .notifiers
            .write()
            .await
            .insert(job.id.clone(), Arc::new(Notify::new()));
        job
    }

    pub async fn mark_running(&self, id: &str) {
        if let Some(job) = self.inner.jobs.write().await.get_mut(id) {
            job.status = BgStatus::Running;
            job.started_at = Some(Utc::now());
        }
    }

    pub async fn mark_done(
        &self,
        id: &str,
        result: String,
        session_id: Option<String>,
        iterations: usize,
    ) {
        if let Some(job) = self.inner.jobs.write().await.get_mut(id) {
            job.status = BgStatus::Done;
            job.finished_at = Some(Utc::now());
            job.result = Some(result);
            job.session_id = session_id;
            job.iterations = iterations;
        }
        self.notify(id).await;
    }

    pub async fn mark_failed(&self, id: &str, error: String) {
        if let Some(job) = self.inner.jobs.write().await.get_mut(id) {
            job.status = BgStatus::Failed;
            job.finished_at = Some(Utc::now());
            job.error = Some(error);
        }
        self.notify(id).await;
    }

    pub async fn mark_cancelled(&self, id: &str) -> bool {
        let mut jobs = self.inner.jobs.write().await;
        let Some(job) = jobs.get_mut(id) else { return false };
        if job.status.is_terminal() {
            return false;
        }
        job.status = BgStatus::Cancelled;
        job.finished_at = Some(Utc::now());
        drop(jobs);
        self.notify(id).await;
        true
    }

    async fn notify(&self, id: &str) {
        if let Some(n) = self.inner.notifiers.read().await.get(id) {
            n.notify_waiters();
        }
    }

    pub async fn get(&self, id: &str) -> Option<BgJob> {
        self.inner.jobs.read().await.get(id).cloned()
    }

    /// Resolve a partial id (the short_id form). Returns the full
    /// matching id when exactly one job matches the prefix, None when
    /// zero or ambiguous matches.
    pub async fn resolve(&self, id_or_prefix: &str) -> Option<String> {
        let jobs = self.inner.jobs.read().await;
        if jobs.contains_key(id_or_prefix) {
            return Some(id_or_prefix.to_string());
        }
        let matches: Vec<String> = jobs
            .keys()
            .filter(|k| k.starts_with(id_or_prefix))
            .cloned()
            .collect();
        if matches.len() == 1 {
            Some(matches.into_iter().next().unwrap())
        } else {
            None
        }
    }

    pub async fn list(&self, include_terminal: bool) -> Vec<BgJob> {
        let jobs = self.inner.jobs.read().await;
        let mut out: Vec<BgJob> = jobs
            .values()
            .filter(|j| include_terminal || !j.status.is_terminal())
            .cloned()
            .collect();
        // Newest first.
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out
    }

    /// Await terminal status for `id`. Returns the final job state.
    pub async fn wait(&self, id: &str, timeout_ms: Option<u64>) -> Option<BgJob> {
        let notifier = {
            let map = self.inner.notifiers.read().await;
            map.get(id).cloned()?
        };
        loop {
            if let Some(job) = self.get(id).await {
                if job.status.is_terminal() {
                    return Some(job);
                }
            } else {
                return None;
            }
            let notified = notifier.notified();
            match timeout_ms {
                Some(ms) => {
                    if tokio::time::timeout(
                        std::time::Duration::from_millis(ms),
                        notified,
                    )
                    .await
                    .is_err()
                    {
                        return self.get(id).await;
                    }
                }
                None => notified.await,
            }
        }
    }

    /// Drop terminal jobs older than the given age. Useful for a
    /// gardener-style cleanup so the in-memory map doesn't grow
    /// unboundedly across a long agent run.
    pub async fn prune_finished(&self, max_age_minutes: i64) -> usize {
        let cutoff = Utc::now() - chrono::Duration::minutes(max_age_minutes);
        let mut jobs = self.inner.jobs.write().await;
        let mut notifiers = self.inner.notifiers.write().await;
        let to_remove: Vec<String> = jobs
            .iter()
            .filter(|(_, j)| {
                j.status.is_terminal()
                    && j.finished_at.map(|t| t < cutoff).unwrap_or(false)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &to_remove {
            jobs.remove(id);
            notifiers.remove(id);
        }
        to_remove.len()
    }
}

impl Default for BgStore {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enqueue_then_get_returns_job() {
        let store = BgStore::new();
        let job = store.enqueue("research X".into(), None).await;
        let fetched = store.get(&job.id).await.unwrap();
        assert_eq!(fetched.task, "research X");
        assert_eq!(fetched.status, BgStatus::Queued);
    }

    #[tokio::test]
    async fn lifecycle_running_done() {
        let store = BgStore::new();
        let job = store.enqueue("t".into(), None).await;
        store.mark_running(&job.id).await;
        assert_eq!(store.get(&job.id).await.unwrap().status, BgStatus::Running);
        store
            .mark_done(&job.id, "result text".into(), Some("sess".into()), 3)
            .await;
        let final_job = store.get(&job.id).await.unwrap();
        assert_eq!(final_job.status, BgStatus::Done);
        assert_eq!(final_job.result.as_deref(), Some("result text"));
        assert_eq!(final_job.iterations, 3);
        assert!(final_job.finished_at.is_some());
    }

    #[tokio::test]
    async fn cancel_only_pre_terminal() {
        let store = BgStore::new();
        let j = store.enqueue("t".into(), None).await;
        assert!(store.mark_cancelled(&j.id).await);
        // second cancel should be no-op
        assert!(!store.mark_cancelled(&j.id).await);
    }

    #[tokio::test]
    async fn resolve_short_id() {
        let store = BgStore::new();
        let j = store.enqueue("t".into(), None).await;
        let short = j.short_id().to_string();
        let resolved = store.resolve(&short).await.expect("should resolve");
        assert_eq!(resolved, j.id);
    }

    #[tokio::test]
    async fn wait_returns_when_done() {
        let store = BgStore::new();
        let j = store.enqueue("t".into(), None).await;
        let id = j.id.clone();
        let store_clone = store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            store_clone
                .mark_done(&id, "ok".into(), None, 1)
                .await;
        });
        let final_job = store.wait(&j.id, Some(500)).await.expect("should resolve");
        assert_eq!(final_job.status, BgStatus::Done);
    }

    #[tokio::test]
    async fn wait_with_timeout_returns_unfinished() {
        let store = BgStore::new();
        let j = store.enqueue("t".into(), None).await;
        let final_job = store
            .wait(&j.id, Some(50))
            .await
            .expect("returns latest snapshot");
        assert_eq!(final_job.status, BgStatus::Queued);
    }

    #[tokio::test]
    async fn list_filters_terminal() {
        let store = BgStore::new();
        let a = store.enqueue("a".into(), None).await;
        let b = store.enqueue("b".into(), None).await;
        store.mark_done(&a.id, "ok".into(), None, 1).await;

        let all = store.list(true).await;
        assert_eq!(all.len(), 2);
        let active = store.list(false).await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, b.id);
    }

    #[tokio::test]
    async fn prune_drops_old_finished() {
        let store = BgStore::new();
        let j = store.enqueue("t".into(), None).await;
        store.mark_done(&j.id, "ok".into(), None, 0).await;
        // Forge an old finished_at by re-marking with a manual write
        // path; for simplicity just call prune with a -1m cutoff so
        // anything finished is in the past from cutoff perspective.
        let removed = store.prune_finished(-60).await;
        assert_eq!(removed, 1);
        assert!(store.get(&j.id).await.is_none());
    }
}
