//! Background job store — fire-and-forget agent jobs with SQLite-backed
//! catalog persistence + per-day usage stats.
//!
//! State model
//!   Queued → Running → (Done | Failed | Cancelled)
//!   Cancellation is cooperative; the spawned worker observes status on
//!   its own polling cycle and quits instead of mid-run abort, which
//!   would corrupt session + brain state.
//!
//! Persistence (Phase 21)
//!   Two SQLite tables under `<data_dir>/bg.db`:
//!     - `bg_jobs`    catalog of every fire ever run, plus a reserved
//!                    `checkpoint_blob` column scoped for Phase 22's
//!                    mid-run resume layer (currently always NULL).
//!     - `bg_stats`   per-day counters keyed by (date, metric). Lets
//!                    the agent introspect its own usage patterns:
//!                    "how often do I exceed budget", "what's my
//!                    average iteration count", etc.
//!
//! On startup, in-flight jobs from a previous process are NOT re-run
//! (we'd need full mid-run resume, deferred). They're simply not
//! present in the in-memory map; the catalog row stays in `Running`
//! until pruned. The user's expected behavior is "fire it again."
//!
//! In-memory map remains the hot path for status queries + waits;
//! SQLite is durable storage. Writes go through both layers
//! (write-through cache pattern). Notifications use the in-memory
//! `Notify` so `wait` is event-driven, not polling SQLite.

use anyhow::Context;
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex as TokioMutex, Notify, RwLock};
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
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
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
    /// Approximate tokens consumed by this job (sum of estimated
    /// per-message token counts). Used for daily-budget bookkeeping.
    pub tokens_used: u64,
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
            tokens_used: 0,
        }
    }

    pub fn short_id(&self) -> &str {
        let n = self.id.len().min(8);
        &self.id[..n]
    }

    pub fn elapsed_ms(&self) -> Option<i64> {
        let end = self.finished_at.unwrap_or_else(Utc::now);
        self.started_at.map(|s| (end - s).num_milliseconds())
    }
}

/// Per-day counters for self-introspection. Metric names are kept
/// open-ended (any string) so future code can add new counters
/// without a schema change.
pub mod metrics {
    pub const FIRED: &str = "fired";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
    pub const CANCELLED: &str = "cancelled";
    pub const TIMEOUT: &str = "timeout";
    pub const BUDGET_REJECTED: &str = "budget_rejected";
    pub const TOKENS_USED: &str = "tokens_used";
    pub const ITERATIONS: &str = "iterations";
    /// Phase 22: a Running job from a prior process resumed from its
    /// checkpoint after agent restart.
    pub const RESUMED: &str = "resumed";
    /// Phase 22: a Running job that had no checkpoint (or stale) was
    /// marked Failed instead of being silently orphaned.
    pub const ORPHANED: &str = "orphaned";
}

/// Phase 22 mid-run resume payload. The bg worker writes one of these
/// after each iteration so the agent can re-spawn the job from where it
/// left off if the process restarts. Encoded with MessagePack
/// (rmp-serde) — compact + schema-tolerant.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub task: String,
    pub iterations: usize,
    pub tokens_so_far: u64,
    pub messages: Vec<crate::message::Message>,
}

impl Checkpoint {
    pub fn encode(&self) -> anyhow::Result<Vec<u8>> {
        rmp_serde::to_vec(self).context("encode checkpoint")
    }
    pub fn decode(blob: &[u8]) -> anyhow::Result<Self> {
        rmp_serde::from_slice(blob).context("decode checkpoint")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DailyStats {
    pub date: String,
    pub fired: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub timeout: u64,
    pub budget_rejected: u64,
    pub tokens_used: u64,
    pub iterations: u64,
    /// Phase 22: jobs resumed from checkpoint after agent restart.
    pub resumed: u64,
    /// Phase 22: Running rows auto-failed because checkpoint was
    /// missing or older than the resume window.
    pub orphaned: u64,
}

/// In-memory + on-disk job store. The struct is cheap to clone (Arc
/// inside) so it can be passed to multiple tools and spawned workers.
#[derive(Clone)]
pub struct BgStore {
    inner: Arc<BgStoreInner>,
}

struct BgStoreInner {
    jobs: RwLock<HashMap<String, BgJob>>,
    notifiers: RwLock<HashMap<String, Arc<Notify>>>,
    db: TokioMutex<Option<Connection>>,
}

impl BgStore {
    /// In-memory only — useful for tests + short-lived processes.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(BgStoreInner {
                jobs: RwLock::new(HashMap::new()),
                notifiers: RwLock::new(HashMap::new()),
                db: TokioMutex::new(None),
            }),
        }
    }

    /// Open a persistent BgStore. The catalog file lives at `db_path`
    /// (typically `<data_dir>/bg.db`). On startup we hydrate the
    /// in-memory map from the catalog; jobs in `Queued` or `Running`
    /// state from a previous process are loaded but NOT re-run —
    /// users `bg fire` them again if they want to retry.
    pub async fn open(db_path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("open bg db at {}", db_path.display()))?;
        Self::init_schema(&conn)?;

        let store = Self::new();
        // Hydrate in-memory map.
        let recovered = Self::load_all(&conn)?;
        {
            let mut jobs = store.inner.jobs.write().await;
            let mut notifiers = store.inner.notifiers.write().await;
            for j in &recovered {
                jobs.insert(j.id.clone(), j.clone());
                notifiers.insert(j.id.clone(), Arc::new(Notify::new()));
            }
        }
        *store.inner.db.lock().await = Some(conn);
        Ok(store)
    }

    fn init_schema(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS bg_jobs (
                id TEXT PRIMARY KEY,
                task TEXT NOT NULL,
                name TEXT,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                started_at TEXT,
                finished_at TEXT,
                result TEXT,
                error TEXT,
                session_id TEXT,
                iterations INTEGER NOT NULL DEFAULT 0,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                checkpoint_blob BLOB,
                checkpoint_at TEXT,
                schema_version INTEGER NOT NULL DEFAULT 1
            );

            CREATE INDEX IF NOT EXISTS idx_bg_status ON bg_jobs(status);
            CREATE INDEX IF NOT EXISTS idx_bg_created ON bg_jobs(created_at DESC);

            CREATE TABLE IF NOT EXISTS bg_stats (
                date TEXT NOT NULL,
                metric TEXT NOT NULL,
                value INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (date, metric)
            );"
        )?;
        Ok(())
    }

    fn load_all(conn: &Connection) -> anyhow::Result<Vec<BgJob>> {
        let mut stmt = conn.prepare(
            "SELECT id, task, name, status, created_at, started_at, finished_at,
                    result, error, session_id, iterations, tokens_used
             FROM bg_jobs ORDER BY created_at DESC LIMIT 500",
        )?;
        let rows = stmt.query_map([], row_to_job)?;
        let mut out = Vec::new();
        for r in rows { out.push(r?); }
        Ok(out)
    }

    async fn write_through<F: FnOnce(&Connection) -> anyhow::Result<()>>(&self, f: F) {
        let db = self.inner.db.lock().await;
        if let Some(conn) = db.as_ref() {
            if let Err(e) = f(conn) {
                tracing::warn!(error = %e, "bg_store: persist failed (in-memory state still updated)");
            }
        }
    }

    /// Increment a counter for today. Best-effort: SQLite errors are
    /// logged at warn but never fail the calling operation.
    pub async fn bump_metric(&self, metric: &str, by: u64) {
        let date = today_key();
        self.write_through(|conn| {
            conn.execute(
                "INSERT INTO bg_stats (date, metric, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(date, metric) DO UPDATE SET value = value + ?3",
                params![date, metric, by as i64],
            )?;
            Ok(())
        })
        .await;
    }

    /// Read today's tokens_used counter. Used by budget checks.
    pub async fn tokens_used_today(&self) -> u64 {
        let db = self.inner.db.lock().await;
        let Some(conn) = db.as_ref() else { return 0 };
        conn.query_row(
            "SELECT value FROM bg_stats WHERE date = ?1 AND metric = ?2",
            params![today_key(), metrics::TOKENS_USED],
            |r| r.get::<_, i64>(0),
        )
        .map(|v| v.max(0) as u64)
        .unwrap_or(0)
    }

    /// Last `days` days of stats, newest first. Includes today even
    /// if no rows exist yet (zero-fill).
    pub async fn recent_stats(&self, days: usize) -> Vec<DailyStats> {
        let db = self.inner.db.lock().await;
        let Some(conn) = db.as_ref() else { return Vec::new() };

        let today = Utc::now().date_naive();
        let mut out: Vec<DailyStats> = Vec::with_capacity(days);
        for offset in 0..days {
            let d = today - chrono::Duration::days(offset as i64);
            let key = d.format("%Y-%m-%d").to_string();
            let mut stats = DailyStats {
                date: key.clone(),
                fired: 0, completed: 0, failed: 0, cancelled: 0,
                timeout: 0, budget_rejected: 0, tokens_used: 0, iterations: 0,
                resumed: 0, orphaned: 0,
            };
            let rows = conn.prepare(
                "SELECT metric, value FROM bg_stats WHERE date = ?1",
            );
            if let Ok(mut stmt) = rows {
                if let Ok(it) = stmt.query_map(params![key], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                }) {
                    for r in it.flatten() {
                        let v = r.1.max(0) as u64;
                        match r.0.as_str() {
                            "fired" => stats.fired = v,
                            "completed" => stats.completed = v,
                            "failed" => stats.failed = v,
                            "cancelled" => stats.cancelled = v,
                            "timeout" => stats.timeout = v,
                            "budget_rejected" => stats.budget_rejected = v,
                            "tokens_used" => stats.tokens_used = v,
                            "iterations" => stats.iterations = v,
                            "resumed" => stats.resumed = v,
                            "orphaned" => stats.orphaned = v,
                            _ => {}
                        }
                    }
                }
            }
            out.push(stats);
        }
        out
    }

    pub async fn enqueue(&self, task: String, name: Option<String>) -> BgJob {
        let job = BgJob::new(task, name);
        self.inner.jobs.write().await.insert(job.id.clone(), job.clone());
        self.inner
            .notifiers
            .write()
            .await
            .insert(job.id.clone(), Arc::new(Notify::new()));
        let j = job.clone();
        self.write_through(move |conn| {
            conn.execute(
                "INSERT INTO bg_jobs (id, task, name, status, created_at, iterations, tokens_used)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
                params![j.id, j.task, j.name, j.status.as_str(), j.created_at.to_rfc3339()],
            )?;
            Ok(())
        })
        .await;
        self.bump_metric(metrics::FIRED, 1).await;
        job
    }

    pub async fn mark_running(&self, id: &str) {
        let now = Utc::now();
        if let Some(job) = self.inner.jobs.write().await.get_mut(id) {
            job.status = BgStatus::Running;
            job.started_at = Some(now);
        }
        let id = id.to_string();
        self.write_through(move |conn| {
            conn.execute(
                "UPDATE bg_jobs SET status = ?1, started_at = ?2 WHERE id = ?3",
                params![BgStatus::Running.as_str(), now.to_rfc3339(), id],
            )?;
            Ok(())
        })
        .await;
    }

    pub async fn mark_done(
        &self,
        id: &str,
        result: String,
        session_id: Option<String>,
        iterations: usize,
        tokens_used: u64,
    ) {
        let now = Utc::now();
        if let Some(job) = self.inner.jobs.write().await.get_mut(id) {
            job.status = BgStatus::Done;
            job.finished_at = Some(now);
            job.result = Some(result.clone());
            job.session_id = session_id.clone();
            job.iterations = iterations;
            job.tokens_used = tokens_used;
        }
        let id_str = id.to_string();
        self.write_through(move |conn| {
            conn.execute(
                "UPDATE bg_jobs SET status = ?1, finished_at = ?2, result = ?3,
                                    session_id = ?4, iterations = ?5, tokens_used = ?6
                 WHERE id = ?7",
                params![BgStatus::Done.as_str(), now.to_rfc3339(), result,
                        session_id, iterations as i64, tokens_used as i64, id_str],
            )?;
            Ok(())
        })
        .await;
        self.bump_metric(metrics::COMPLETED, 1).await;
        self.bump_metric(metrics::TOKENS_USED, tokens_used).await;
        self.bump_metric(metrics::ITERATIONS, iterations as u64).await;
        self.notify(id).await;
    }

    pub async fn mark_failed(&self, id: &str, error: String) {
        let now = Utc::now();
        if let Some(job) = self.inner.jobs.write().await.get_mut(id) {
            job.status = BgStatus::Failed;
            job.finished_at = Some(now);
            job.error = Some(error.clone());
        }
        let id_str = id.to_string();
        self.write_through(move |conn| {
            conn.execute(
                "UPDATE bg_jobs SET status = ?1, finished_at = ?2, error = ?3 WHERE id = ?4",
                params![BgStatus::Failed.as_str(), now.to_rfc3339(), error, id_str],
            )?;
            Ok(())
        })
        .await;
        self.bump_metric(metrics::FAILED, 1).await;
        self.notify(id).await;
    }

    pub async fn mark_timeout(&self, id: &str, secs: u64) {
        self.bump_metric(metrics::TIMEOUT, 1).await;
        self.mark_failed(id, format!("timeout: exceeded {secs}s")).await;
    }

    /// Phase 22: persist the bg worker's mid-run state. Called after
    /// each iteration so the job can be resumed across restarts. Writes
    /// `checkpoint_blob` + `checkpoint_at` only; status stays `Running`.
    pub async fn write_checkpoint(&self, id: &str, ckpt: &Checkpoint) -> anyhow::Result<()> {
        let blob = ckpt.encode()?;
        let now = Utc::now().to_rfc3339();
        let id_str = id.to_string();
        // Mirror the in-memory iteration count so `bg status` reflects
        // checkpoint progress even for jobs nobody waits on.
        if let Some(job) = self.inner.jobs.write().await.get_mut(id) {
            job.iterations = ckpt.iterations;
        }
        self.write_through(move |conn| {
            conn.execute(
                "UPDATE bg_jobs SET checkpoint_blob = ?1, checkpoint_at = ?2,
                                    iterations = ?3
                 WHERE id = ?4",
                params![blob, now, ckpt.iterations as i64, id_str],
            )?;
            Ok(())
        })
        .await;
        Ok(())
    }

    /// Phase 22: scan for `Running` jobs left behind by a prior agent
    /// process. Returns (job, decoded checkpoint) for jobs whose
    /// checkpoint is fresh enough (`max_age` window); jobs older than
    /// the window are auto-marked failed via `ORPHANED` so they don't
    /// pollute Running forever.
    pub async fn pending_resumable(
        &self,
        max_age: chrono::Duration,
    ) -> Vec<(BgJob, Checkpoint)> {
        let cutoff = Utc::now() - max_age;
        let cutoff_str = cutoff.to_rfc3339();
        let mut out: Vec<(BgJob, Checkpoint)> = Vec::new();
        let mut to_orphan: Vec<(String, String)> = Vec::new();

        // Snapshot rows from disk (in-memory map already mirrors them).
        let rows: Vec<(String, Option<Vec<u8>>, Option<String>)> = {
            let db = self.inner.db.lock().await;
            let Some(conn) = db.as_ref() else { return out };
            let mut stmt = match conn.prepare(
                "SELECT id, checkpoint_blob, checkpoint_at FROM bg_jobs
                 WHERE status = 'running'",
            ) {
                Ok(s) => s,
                Err(_) => return out,
            };
            let mapped = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<Vec<u8>>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            });
            match mapped {
                Ok(it) => it.filter_map(|r| r.ok()).collect(),
                Err(_) => Vec::new(),
            }
        };

        for (id, blob, ckpt_at) in rows {
            let too_old = ckpt_at
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc) < cutoff)
                .unwrap_or(true); // no checkpoint_at ⇒ treat as too old / never checkpointed
            let Some(blob) = blob else {
                to_orphan.push((id, "no checkpoint written before restart".into()));
                continue;
            };
            if too_old {
                to_orphan.push((id, format!("checkpoint older than {} hours", max_age.num_hours())));
                continue;
            }
            let Ok(ckpt) = Checkpoint::decode(&blob) else {
                to_orphan.push((id, "checkpoint decode failed (schema drift?)".into()));
                continue;
            };
            if let Some(job) = self.get(&id).await {
                out.push((job, ckpt));
            }
        }

        for (id, reason) in to_orphan {
            self.bump_metric(metrics::ORPHANED, 1).await;
            self.mark_failed(&id, reason).await;
            // mark_failed already bumps FAILED — we accept the double
            // count (orphaned ⊆ failed) as deliberate categorisation.
        }

        out
    }

    pub async fn mark_cancelled(&self, id: &str) -> bool {
        let now = Utc::now();
        let mut jobs = self.inner.jobs.write().await;
        let Some(job) = jobs.get_mut(id) else { return false };
        if job.status.is_terminal() {
            return false;
        }
        job.status = BgStatus::Cancelled;
        job.finished_at = Some(now);
        drop(jobs);
        let id_str = id.to_string();
        self.write_through(move |conn| {
            conn.execute(
                "UPDATE bg_jobs SET status = ?1, finished_at = ?2 WHERE id = ?3",
                params![BgStatus::Cancelled.as_str(), now.to_rfc3339(), id_str],
            )?;
            Ok(())
        })
        .await;
        self.bump_metric(metrics::CANCELLED, 1).await;
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

    pub async fn resolve(&self, id_or_prefix: &str) -> Option<String> {
        let jobs = self.inner.jobs.read().await;
        if jobs.contains_key(id_or_prefix) { return Some(id_or_prefix.to_string()); }
        let m: Vec<String> = jobs
            .keys()
            .filter(|k| k.starts_with(id_or_prefix))
            .cloned()
            .collect();
        if m.len() == 1 { Some(m.into_iter().next().unwrap()) } else { None }
    }

    pub async fn list(&self, include_terminal: bool) -> Vec<BgJob> {
        let jobs = self.inner.jobs.read().await;
        let mut out: Vec<BgJob> = jobs
            .values()
            .filter(|j| include_terminal || !j.status.is_terminal())
            .cloned()
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out
    }

    pub async fn wait(&self, id: &str, timeout_ms: Option<u64>) -> Option<BgJob> {
        let notifier = {
            let map = self.inner.notifiers.read().await;
            map.get(id).cloned()?
        };
        loop {
            if let Some(job) = self.get(id).await {
                if job.status.is_terminal() { return Some(job); }
            } else { return None; }
            let notified = notifier.notified();
            match timeout_ms {
                Some(ms) => {
                    if tokio::time::timeout(
                        std::time::Duration::from_millis(ms),
                        notified,
                    ).await.is_err() {
                        return self.get(id).await;
                    }
                }
                None => notified.await,
            }
        }
    }

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
        let cutoff_str = cutoff.to_rfc3339();
        self.write_through(move |conn| {
            conn.execute(
                "DELETE FROM bg_jobs WHERE status IN ('done','failed','cancelled')
                 AND finished_at IS NOT NULL AND finished_at < ?1",
                params![cutoff_str],
            )?;
            Ok(())
        })
        .await;
        to_remove.len()
    }
}

impl Default for BgStore {
    fn default() -> Self { Self::new() }
}

fn row_to_job(row: &rusqlite::Row) -> rusqlite::Result<BgJob> {
    let parse_dt = |s: Option<String>| -> Option<DateTime<Utc>> {
        s.and_then(|t| DateTime::parse_from_rfc3339(&t).ok().map(|d| d.with_timezone(&Utc)))
    };
    let status_str: String = row.get(3)?;
    let created_str: String = row.get(4)?;
    let created_at = DateTime::parse_from_rfc3339(&created_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Ok(BgJob {
        id: row.get(0)?,
        task: row.get(1)?,
        name: row.get(2)?,
        status: BgStatus::parse(&status_str).unwrap_or(BgStatus::Failed),
        created_at,
        started_at: parse_dt(row.get(5)?),
        finished_at: parse_dt(row.get(6)?),
        result: row.get(7)?,
        error: row.get(8)?,
        session_id: row.get(9)?,
        iterations: row.get::<_, i64>(10).unwrap_or(0).max(0) as usize,
        tokens_used: row.get::<_, i64>(11).unwrap_or(0).max(0) as u64,
    })
}

fn today_key() -> String {
    Utc::now().date_naive().format("%Y-%m-%d").to_string()
}

/// Estimate tokens for a finished agent task. Loose heuristic: input
/// tokens ~ task.len() / 4, plus per-iteration overhead. Used for
/// daily-budget bookkeeping when the LLM provider doesn't return
/// usage stats. Documented as approximate; users should configure
/// `[bg].max_tokens_per_day` with headroom.
pub fn estimate_tokens(task: &str, iterations: usize, response_text: &str) -> u64 {
    let task_tokens = (task.chars().count() / 4) as u64;
    let response_tokens = (response_text.chars().count() / 4) as u64;
    // System prompt + intermediate tool calls add up; rough estimate
    // is ~800 tokens per iteration round-trip on this codebase.
    let iteration_tokens = (iterations as u64) * 800;
    task_tokens + response_tokens + iteration_tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lifecycle_and_persistence() {
        let dir = std::env::temp_dir().join(format!("peko-bg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("bg.db");

        let store = BgStore::open(&db).await.unwrap();
        let j = store.enqueue("research X".into(), Some("r1".into())).await;
        store.mark_running(&j.id).await;
        store
            .mark_done(&j.id, "summary".into(), Some("sess".into()), 5, 12345)
            .await;

        // Recreate store from disk.
        let store2 = BgStore::open(&db).await.unwrap();
        let recovered = store2.get(&j.id).await.unwrap();
        assert_eq!(recovered.status, BgStatus::Done);
        assert_eq!(recovered.iterations, 5);
        assert_eq!(recovered.tokens_used, 12345);
        assert_eq!(recovered.result.as_deref(), Some("summary"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn stats_accumulate_per_day() {
        let store = BgStore::new();
        // Without a DB, bump_metric is a no-op; use the persistent path.
        let dir = std::env::temp_dir().join(format!("peko-bg-stats-{}", rand_token()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = BgStore::open(&dir.join("bg.db")).await.unwrap();

        let _ = store.enqueue("a".into(), None).await;
        let _ = store.enqueue("b".into(), None).await;

        let recent = store.recent_stats(1).await;
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].fired, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cancel_only_pre_terminal() {
        let store = BgStore::new();
        let j = store.enqueue("t".into(), None).await;
        assert!(store.mark_cancelled(&j.id).await);
        assert!(!store.mark_cancelled(&j.id).await);
    }

    #[tokio::test]
    async fn resolve_short_id_unique() {
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
            store_clone.mark_done(&id, "ok".into(), None, 1, 0).await;
        });
        let final_job = store.wait(&j.id, Some(500)).await.expect("resolves");
        assert_eq!(final_job.status, BgStatus::Done);
    }

    #[tokio::test]
    async fn wait_with_timeout_returns_unfinished() {
        let store = BgStore::new();
        let j = store.enqueue("t".into(), None).await;
        let final_job = store.wait(&j.id, Some(50)).await.expect("returns snapshot");
        assert_eq!(final_job.status, BgStatus::Queued);
    }

    #[tokio::test]
    async fn list_filters_terminal() {
        let store = BgStore::new();
        let a = store.enqueue("a".into(), None).await;
        let b = store.enqueue("b".into(), None).await;
        store.mark_done(&a.id, "ok".into(), None, 1, 0).await;
        let all = store.list(true).await;
        assert_eq!(all.len(), 2);
        let active = store.list(false).await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, b.id);
    }

    #[tokio::test]
    async fn estimate_tokens_grows_with_iterations() {
        let t1 = estimate_tokens("hi", 1, "ok");
        let t10 = estimate_tokens("hi", 10, "ok");
        assert!(t10 > t1, "more iterations should estimate more tokens");
    }

    #[tokio::test]
    async fn tokens_used_today_reflects_marks() {
        let dir = std::env::temp_dir().join(format!("peko-bg-toks-{}", rand_token()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = BgStore::open(&dir.join("bg.db")).await.unwrap();
        let j = store.enqueue("t".into(), None).await;
        store.mark_done(&j.id, "ok".into(), None, 2, 1500).await;
        let used = store.tokens_used_today().await;
        assert_eq!(used, 1500);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn checkpoint_roundtrip_and_resume_window() {
        let dir = std::env::temp_dir().join(format!("peko-bg-ckpt-{}", rand_token()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("bg.db");
        let store = BgStore::open(&db).await.unwrap();

        // Two running jobs: A has a fresh checkpoint, B has none.
        let a = store.enqueue("research a".into(), None).await;
        let b = store.enqueue("research b".into(), None).await;
        store.mark_running(&a.id).await;
        store.mark_running(&b.id).await;

        let ckpt = Checkpoint {
            task: "research a".into(),
            iterations: 3,
            tokens_so_far: 1234,
            messages: vec![crate::message::Message::user("hi".to_string())],
        };
        store.write_checkpoint(&a.id, &ckpt).await.unwrap();

        let resumable = store.pending_resumable(chrono::Duration::hours(1)).await;
        assert_eq!(resumable.len(), 1, "only A is resumable");
        assert_eq!(resumable[0].0.id, a.id);
        assert_eq!(resumable[0].1.iterations, 3);

        // B should have been auto-orphaned (marked failed).
        let b_after = store.get(&b.id).await.unwrap();
        assert_eq!(b_after.status, BgStatus::Failed);
        assert!(b_after.error.as_deref().unwrap_or("").contains("no checkpoint"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn prune_drops_old_finished() {
        let store = BgStore::new();
        let j = store.enqueue("t".into(), None).await;
        store.mark_done(&j.id, "ok".into(), None, 0, 0).await;
        let removed = store.prune_finished(-60).await;
        assert_eq!(removed, 1);
        assert!(store.get(&j.id).await.is_none());
    }

    fn rand_token() -> String {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        format!("{pid}-{nanos}")
    }
}

// Suppress dead-code warning for the in-memory-only Mutex import when
// the module is compiled without `tokio::sync::Mutex` direct refs.
#[allow(dead_code)]
fn _unused_paths_check(_: PathBuf) {}
