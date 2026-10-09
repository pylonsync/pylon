//! Background job queue with priority scheduling, retries, and dead-letter support.
//!
//! Jobs are enqueued with a name, JSON payload, priority, and retry policy.
//! Workers pull from the queue and invoke registered handlers. Failed jobs are
//! retried with exponential back-off until `max_retries` is exhausted, at which
//! point they move to the dead-letter queue for manual inspection.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Job priority levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Priority {
    /// Parse a priority from a string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "low" => Self::Low,
            "high" => Self::High,
            "critical" => Self::Critical,
            _ => Self::Normal,
        }
    }
}

/// Job status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Retrying,
    Dead,
    /// Removed from the queue by `ctx.scheduler.cancel` before it ran.
    /// Terminal: restore and claim paths never pick it up again.
    Cancelled,
}

impl JobStatus {
    /// The status string stored in the job tables and accepted by the
    /// job listing filters.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Retrying => "retrying",
            Self::Dead => "dead",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parse a stored status string. Unknown strings map to `Pending`.
    pub fn from_stored(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "retrying" => Self::Retrying,
            "dead" => Self::Dead,
            "cancelled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }
}

/// Auth context propagated from the function that scheduled this job.
///
/// Scheduled function jobs run with the identity of the caller that
/// invoked `ctx.scheduler.runAfter/runAt` — same semantics as a direct
/// call. Without this, every scheduled callback ran with anonymous
/// auth (`user_id: None, is_admin: false`), which broke any app whose
/// internal mutations reject anonymous callers as a defense against
/// direct HTTP smuggling. The smuggle-gate already enforces chain-of-
/// custody at schedule time (only admins or internal:true callers may
/// enqueue internal:true targets), so propagating the caller's
/// identity to the job is consistent with that gate — not a new
/// privilege-escalation surface.
///
/// `None` on a Job means "default anonymous" — used for the framework's
/// built-in cron jobs (`pylon.cache.cleanup`, `pylon.streams.cleanup`)
/// and any caller that explicitly opted out.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobAuth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    /// The scheduling caller was an anonymous guest session. Defaults to
    /// false, so a job persisted before this field existed deserializes
    /// with the behaviour it was enqueued under.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_guest: bool,
}

/// A job in the queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub payload: serde_json::Value,
    pub priority: Priority,
    pub status: JobStatus,
    pub max_retries: u32,
    pub retry_count: u32,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub error: Option<String>,
    /// Delay before first execution (in seconds from creation).
    pub delay_secs: u64,
    /// Absolute epoch-seconds before which the job must not run. 0 means
    /// unset — readiness falls back to `created_at + delay_secs`. Stamped
    /// by `fail()` with the retry back-off; `delay_secs`/`created_at` are
    /// left alone so the job's creation time stays truthful in listings.
    /// `#[serde(default)]` keeps older persisted jobs deserializable.
    #[serde(default)]
    pub ready_at: u64,
    /// Queue name (for routing to specific workers).
    pub queue: String,
    /// Auth identity to dispatch the handler with. `None` falls back
    /// to anonymous (no user, not admin). `#[serde(default)]` keeps
    /// older persisted jobs deserializable after this field landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<JobAuth>,
}

/// Result of processing a job.
pub enum JobResult {
    Success,
    Failure(String),
    Retry(String),
}

/// A handler function for a named job type.
pub type JobHandler = Arc<dyn Fn(&Job) -> JobResult + Send + Sync>;

// ---------------------------------------------------------------------------
// Queue statistics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct QueueStats {
    pub pending: usize,
    pub running: usize,
    pub completed: u64,
    pub failed: u64,
    pub dead: usize,
    pub handlers: Vec<String>,
}

// ---------------------------------------------------------------------------
// JobQueue
// ---------------------------------------------------------------------------

/// The job queue engine.
///
/// Thread-safe: all internal state is behind mutexes, and a `Condvar` wakes
/// blocked workers when new jobs arrive.
pub struct JobQueue {
    /// Pending jobs, sorted by insertion with priority taken into account
    /// during dequeue.
    pending: Mutex<VecDeque<Job>>,
    /// Running jobs (id -> job).
    running: Mutex<HashMap<String, Job>>,
    /// Completed/failed job history (bounded ring buffer).
    history: Mutex<VecDeque<Job>>,
    /// Registered handlers by job name.
    handlers: Mutex<HashMap<String, JobHandler>>,
    /// Signal for workers to wake up when jobs are available.
    notify: Condvar,
    /// Maximum history entries to retain.
    max_history: usize,
    /// Dead letter queue.
    dead_letters: Mutex<VecDeque<Job>>,
    /// Monotonic counters for stats.
    completed_count: AtomicU64,
    failed_count: AtomicU64,
    /// Monotonic ID counter.
    next_id: AtomicU64,
    /// Per-process prefix for globally unique job and lease ids.
    instance_id: String,
    /// Optional persistent backing store. When set, every state transition is
    /// mirrored to SQLite so jobs survive restart. Failures to persist are
    /// logged but not surfaced — durability is best-effort, never blocking.
    store: Mutex<Option<std::sync::Arc<crate::job_store::JobStore>>>,
    /// Shared Postgres store. When present, Postgres is the queue itself.
    /// Pending jobs are not copied into every process.
    pg_store: Mutex<Option<std::sync::Arc<crate::pg_job_store::PgJobStore>>>,
    /// Lease token for each job this process currently executes.
    lease_tokens: Mutex<HashMap<String, String>>,
    /// First-retry back-off in seconds (doubles per attempt, capped at
    /// [`MAX_RETRY_BACKOFF_SECS`]). Default 1. Tests set 0 to keep the
    /// fail→dequeue→fail loop synchronous.
    retry_backoff_base_secs: AtomicU64,
}

impl JobQueue {
    pub fn new(max_history: usize) -> Self {
        Self {
            pending: Mutex::new(VecDeque::new()),
            running: Mutex::new(HashMap::new()),
            history: Mutex::new(VecDeque::new()),
            handlers: Mutex::new(HashMap::new()),
            notify: Condvar::new(),
            max_history,
            dead_letters: Mutex::new(VecDeque::new()),
            completed_count: AtomicU64::new(0),
            failed_count: AtomicU64::new(0),
            next_id: AtomicU64::new(1),
            instance_id: pylon_cluster::new_instance_id(),
            store: Mutex::new(None),
            pg_store: Mutex::new(None),
            lease_tokens: Mutex::new(HashMap::new()),
            retry_backoff_base_secs: AtomicU64::new(1),
        }
    }

    /// Override the first-retry back-off (seconds). 0 disables the delay —
    /// tests use it so retries are immediately dequeuable.
    pub fn set_retry_backoff_base_secs(&self, secs: u64) {
        self.retry_backoff_base_secs.store(secs, Ordering::Relaxed);
    }

    /// Attach a persistent store. After this, every enqueue, state change, and
    /// terminal event is mirrored to the store. Call once at startup.
    pub fn attach_store(&self, store: std::sync::Arc<crate::job_store::JobStore>) {
        *self.store.lock().unwrap() = Some(store);
    }

    /// Attach the shared Postgres queue. A distributed queue never restores
    /// rows into process memory. Workers claim rows directly from Postgres.
    pub fn attach_pg_store(&self, store: std::sync::Arc<crate::pg_job_store::PgJobStore>) {
        *self.pg_store.lock().unwrap() = Some(store);
    }

    fn pg_store(&self) -> Option<std::sync::Arc<crate::pg_job_store::PgJobStore>> {
        self.pg_store.lock().unwrap().clone()
    }

    pub fn is_distributed(&self) -> bool {
        self.pg_store.lock().unwrap().is_some()
    }

    /// Best-effort persist. Never panics, never propagates errors — durability
    /// is opportunistic. If the store is detached or write fails, logs and
    /// continues.
    fn persist(&self, job: &Job) {
        if let Some(store) = self.store.lock().unwrap().as_ref() {
            if let Err(e) = store.save(job) {
                tracing::warn!("[jobs] failed to persist job {}: {e}", job.id);
            }
        }
    }

    /// Prune completed/dead job rows older than `max_age_secs` from the backing
    /// store; returns the number deleted (0 if no store is attached).
    ///
    /// Without this the jobs table grows unbounded: every run of the built-in
    /// recurring jobs (`pylon.cache.cleanup`, `pylon.streams.cleanup`,
    /// `pylon.ratelimit.cleanup`) leaves a `completed` row forever. On a
    /// long-lived app that's thousands of rows + a multi-MB WAL, which slowed
    /// boot (the store is read + WAL-recovered before the HTTP listener binds).
    /// `JobStore::cleanup_completed` existed but nothing called it.
    pub fn cleanup_completed_jobs(&self, max_age_secs: u64) -> usize {
        if let Some(store) = self.pg_store() {
            return store.cleanup_completed(max_age_secs).unwrap_or_else(|e| {
                tracing::warn!("[jobs] Postgres cleanup failed: {e}");
                0
            });
        }
        if let Some(store) = self.store.lock().unwrap().as_ref() {
            store.cleanup_completed(max_age_secs)
        } else {
            0
        }
    }

    /// Register a handler for a job type.
    pub fn register(&self, job_name: &str, handler: JobHandler) {
        self.handlers
            .lock()
            .unwrap()
            .insert(job_name.to_string(), handler);
    }

    /// Enqueue a new job with default options. Returns the job ID.
    pub fn enqueue(&self, name: &str, payload: serde_json::Value) -> String {
        self.enqueue_with_options(name, payload, Priority::Normal, 0, 3, "default")
    }

    /// Enqueue with full options. Returns the job id, or an empty string if
    /// the persistent store rejected the write. Prefer
    /// [`try_enqueue_with_options`] in new code so persist failures don't
    /// look like success to the caller.
    pub fn enqueue_with_options(
        &self,
        name: &str,
        payload: serde_json::Value,
        priority: Priority,
        delay_secs: u64,
        max_retries: u32,
        queue: &str,
    ) -> String {
        match self.try_enqueue_with_options(name, payload, priority, delay_secs, max_retries, queue)
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("[jobs] enqueue rejected: {e}");
                String::new()
            }
        }
    }

    /// Result-returning variant of [`enqueue_with_options`]. Use this from
    /// any path where a silent failure would propagate as an apparent
    /// success (e.g. the TS scheduler hook returning `id: ""` to the user).
    pub fn try_enqueue_with_options(
        &self,
        name: &str,
        payload: serde_json::Value,
        priority: Priority,
        delay_secs: u64,
        max_retries: u32,
        queue: &str,
    ) -> Result<String, String> {
        self.try_enqueue_with_auth(
            name,
            payload,
            priority,
            delay_secs,
            max_retries,
            queue,
            None,
        )
    }

    /// Enqueue with an explicit auth identity. The worker dispatches
    /// the handler with this identity — used by the function-scheduler
    /// hook so a scheduled callback runs as the function that scheduled
    /// it, not as anonymous.
    #[allow(clippy::too_many_arguments)]
    pub fn try_enqueue_with_auth(
        &self,
        name: &str,
        payload: serde_json::Value,
        priority: Priority,
        delay_secs: u64,
        max_retries: u32,
        queue: &str,
        auth: Option<JobAuth>,
    ) -> Result<String, String> {
        let job = self.new_job(
            name,
            payload,
            priority,
            delay_secs,
            max_retries,
            queue,
            auth,
        );
        self.try_enqueue_job(job)
    }

    /// Insert a prebuilt job through the mutation's held Postgres
    /// transaction. The job row and the application writes then commit or
    /// roll back together.
    pub(crate) fn enqueue_job_in_transaction(
        &self,
        tx_store: &dyn pylon_http::DataStore,
        job: Job,
    ) -> Result<String, String> {
        if !self.is_distributed() {
            return Err("transactional durable enqueue requires the Postgres job store".into());
        }
        let encoded = serde_json::to_value(&job)
            .map_err(|e| format!("failed to encode scheduled job: {e}"))?;
        tx_store
            .enqueue_internal_job(&encoded)
            .map_err(|e| format!("{}: {}", e.code, e.message))?;
        Ok(job.id)
    }

    /// Build a pending job with a fresh id without enqueuing it. The id
    /// carries this process's instance id, so it never repeats the id of a
    /// job an earlier process left in the job store.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_job(
        &self,
        name: &str,
        payload: serde_json::Value,
        priority: Priority,
        delay_secs: u64,
        max_retries: u32,
        queue: &str,
        auth: Option<JobAuth>,
    ) -> Job {
        let sequence = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("job_{}_{sequence}", self.instance_id);
        let now = now_iso();
        Job {
            id: id.clone(),
            name: name.to_string(),
            payload,
            priority,
            status: JobStatus::Pending,
            max_retries,
            retry_count: 0,
            created_at: now,
            started_at: None,
            completed_at: None,
            error: None,
            delay_secs,
            ready_at: 0,
            queue: queue.to_string(),
            auth,
        }
    }

    pub(crate) fn try_enqueue_job(&self, job: Job) -> Result<String, String> {
        if let Some(store) = self.pg_store() {
            store.enqueue(&job)?;
            self.notify.notify_one();
            return Ok(job.id);
        }
        // Write-ahead: persist BEFORE the in-memory queue accepts the job, so
        // a crash between the two states can't lose an accepted job.
        if let Some(store) = self.store.lock().unwrap().as_ref() {
            if let Err(e) = store.save(&job) {
                return Err(format!("persist failed for job {}: {e}", job.id));
            }
        }

        let id = job.id.clone();
        let priority = job.priority;
        {
            let mut pending = self.pending.lock().unwrap();
            // Insert in priority order (higher priority closer to front).
            let pos = pending.partition_point(|j| (j.priority as u8) >= (priority as u8));
            pending.insert(pos, job);
        }
        self.notify.notify_one();
        Ok(id)
    }

    /// Dequeue the highest-priority pending job whose `delay_secs` has
    /// elapsed. Blocks up to `timeout` if nothing is ready.
    pub fn dequeue(&self, timeout: Duration) -> Option<Job> {
        if let Some(store) = self.pg_store() {
            let handlers: Vec<String> = self.handlers.lock().unwrap().keys().cloned().collect();
            match store.claim(None, &handlers, DISTRIBUTED_LEASE_SECS) {
                Ok(Some((job, token))) => {
                    self.lease_tokens
                        .lock()
                        .unwrap()
                        .insert(job.id.clone(), token);
                    self.running
                        .lock()
                        .unwrap()
                        .insert(job.id.clone(), job.clone());
                    return Some(job);
                }
                Ok(None) => self.wait_for_work(timeout),
                Err(e) => {
                    tracing::warn!("[jobs] Postgres claim failed: {e}");
                    self.wait_for_work(timeout);
                }
            }
            return None;
        }
        let mut pending = self.pending.lock().unwrap();
        let now = now_secs();
        if !pending.iter().any(|j| is_ready(j, now)) {
            let (guard, _) = self.notify.wait_timeout(pending, timeout).unwrap();
            pending = guard;
        }

        let now = now_secs();
        let pos = pending.iter().position(|j| is_ready(j, now));
        if let Some(idx) = pos {
            let mut job = pending.remove(idx).unwrap();
            job.status = JobStatus::Running;
            job.started_at = Some(now_iso());
            self.running
                .lock()
                .unwrap()
                .insert(job.id.clone(), job.clone());
            self.persist(&job);
            Some(job)
        } else {
            None
        }
    }

    /// Dequeue from a specific queue. Blocks up to `timeout` if nothing
    /// in the queue is ready (delay-respecting).
    pub fn dequeue_from(&self, queue: &str, timeout: Duration) -> Option<Job> {
        if let Some(store) = self.pg_store() {
            let handlers: Vec<String> = self.handlers.lock().unwrap().keys().cloned().collect();
            match store.claim(Some(queue), &handlers, DISTRIBUTED_LEASE_SECS) {
                Ok(Some((job, token))) => {
                    self.lease_tokens
                        .lock()
                        .unwrap()
                        .insert(job.id.clone(), token);
                    self.running
                        .lock()
                        .unwrap()
                        .insert(job.id.clone(), job.clone());
                    return Some(job);
                }
                Ok(None) => self.wait_for_work(timeout),
                Err(e) => {
                    tracing::warn!("[jobs] Postgres queue claim failed: {e}");
                    self.wait_for_work(timeout);
                }
            }
            return None;
        }
        let mut pending = self.pending.lock().unwrap();
        let now = now_secs();
        if !pending.iter().any(|j| j.queue == queue && is_ready(j, now)) {
            let (guard, _) = self.notify.wait_timeout(pending, timeout).unwrap();
            pending = guard;
        }

        let now = now_secs();
        let pos = pending
            .iter()
            .position(|j| j.queue == queue && is_ready(j, now));
        if let Some(idx) = pos {
            let mut job = pending.remove(idx).unwrap();
            job.status = JobStatus::Running;
            job.started_at = Some(now_iso());
            self.running
                .lock()
                .unwrap()
                .insert(job.id.clone(), job.clone());
            self.persist(&job);
            Some(job)
        } else {
            None
        }
    }

    /// Mark a job as completed.
    pub fn complete(&self, job_id: &str) {
        let job = self.running.lock().unwrap().remove(job_id);
        if let Some(mut job) = job {
            if let Some(store) = self.pg_store() {
                let token = self.lease_tokens.lock().unwrap().remove(job_id);
                match token {
                    Some(token) => match store.complete(job_id, &token) {
                        Ok(true) => {
                            self.completed_count.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(false) => tracing::warn!(
                            "[jobs] completion rejected for {job_id}: lease is no longer owned"
                        ),
                        Err(e) => tracing::warn!("[jobs] completion failed for {job_id}: {e}"),
                    },
                    None => tracing::warn!("[jobs] completion missing lease token for {job_id}"),
                }
                return;
            }
            job.status = JobStatus::Completed;
            job.completed_at = Some(now_iso());
            self.completed_count.fetch_add(1, Ordering::Relaxed);
            self.persist(&job);
            self.push_history(job);
        }
    }

    /// Mark a job as failed. Retries with exponential back-off if under
    /// max_retries: base × 2^(attempt-1), capped at [`MAX_RETRY_BACKOFF_SECS`].
    /// Without the back-off stamp, a deterministic failure burned every
    /// retry in milliseconds and dead-lettered before the operator could
    /// blink — the exact opposite of what retries are for.
    pub fn fail(&self, job_id: &str, error: &str) {
        let job = self.running.lock().unwrap().remove(job_id);
        if let Some(mut job) = job {
            if let Some(store) = self.pg_store() {
                let token = self.lease_tokens.lock().unwrap().remove(job_id);
                let next_retry = job.retry_count.saturating_add(1);
                let ready_at = now_secs().saturating_add(retry_backoff_secs(
                    self.retry_backoff_base_secs.load(Ordering::Relaxed),
                    next_retry,
                ));
                match token {
                    Some(token) => match store.fail(&job, &token, error, ready_at) {
                        Ok(Some(JobStatus::Dead)) => {
                            self.failed_count.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(Some(_)) => self.notify.notify_one(),
                        Ok(None) => tracing::warn!(
                            "[jobs] failure update rejected for {job_id}: lease is no longer owned"
                        ),
                        Err(e) => tracing::warn!("[jobs] failure update failed for {job_id}: {e}"),
                    },
                    None => tracing::warn!("[jobs] failure missing lease token for {job_id}"),
                }
                return;
            }
            job.error = Some(error.to_string());

            if job.retry_count < job.max_retries {
                // Re-enqueue for retry.
                job.retry_count += 1;
                job.status = JobStatus::Retrying;
                job.started_at = None;
                job.completed_at = None;
                job.ready_at = now_secs().saturating_add(retry_backoff_secs(
                    self.retry_backoff_base_secs.load(Ordering::Relaxed),
                    job.retry_count,
                ));

                self.persist(&job);
                let mut pending = self.pending.lock().unwrap();
                let priority = job.priority as u8;
                let pos = pending.partition_point(|j| (j.priority as u8) >= priority);
                pending.insert(pos, job);
                drop(pending);
                self.notify.notify_one();
            } else {
                // Exhausted retries -- move to dead letter queue.
                job.status = JobStatus::Dead;
                job.completed_at = Some(now_iso());
                self.failed_count.fetch_add(1, Ordering::Relaxed);
                self.persist(&job);
                self.dead_letters.lock().unwrap().push_back(job);
            }
        }
    }

    /// Cancel a job that has not started. Returns `Ok(true)` when the job
    /// was pending or waiting to retry and is now `Cancelled` in the store.
    /// Returns `Ok(false)` when the id is unknown or the job is running or
    /// already finished; the job is left unchanged. Returns `Err` when the
    /// cancelled state could not be persisted; the job then stays queued.
    ///
    /// A worker claims a job and this method cancels it under the same
    /// lock (the in-memory `pending` mutex, or the Postgres row lock), so
    /// a job is either claimed or cancelled, never both.
    pub fn cancel_pending(&self, job_id: &str) -> Result<bool, String> {
        if let Some(store) = self.pg_store() {
            return store.cancel(job_id);
        }
        Ok(self.cancel_pending_local(job_id)?.is_some())
    }

    /// In-memory half of [`Self::cancel_pending`]. Returns the job as it
    /// was before the cancel, so a rolled-back mutation can hand it to
    /// [`Self::restore_cancelled`].
    pub(crate) fn cancel_pending_local(&self, job_id: &str) -> Result<Option<Job>, String> {
        let mut pending = self.pending.lock().unwrap();
        let Some(idx) = pending.iter().position(|j| j.id == job_id) else {
            return Ok(None);
        };
        let mut cancelled = pending[idx].clone();
        cancelled.status = JobStatus::Cancelled;
        cancelled.completed_at = Some(now_iso());
        // Persist before the job leaves the queue, while the `pending`
        // lock is held: a persist failure changes nothing, and no worker
        // can claim the job in between.
        if let Some(store) = self.store.lock().unwrap().as_ref() {
            store
                .save(&cancelled)
                .map_err(|e| format!("persist failed for job {job_id}: {e}"))?;
        }
        let original = pending
            .remove(idx)
            .expect("index found under the same lock");
        drop(pending);
        self.push_history(cancelled);
        Ok(Some(original))
    }

    /// Put back a job that [`Self::cancel_pending_local`] cancelled inside
    /// a mutation that then rolled back. The job returns to the queue and
    /// the store with its earlier status. A store failure is logged and
    /// the job is queued anyway: it runs in this process, but a restart
    /// before it runs would drop it.
    pub(crate) fn restore_cancelled(&self, original: Job) {
        if let Some(store) = self.store.lock().unwrap().as_ref() {
            if let Err(e) = store.save(&original) {
                tracing::error!(
                    "[jobs] could not restore job {} after a rolled-back cancel: {e}",
                    original.id
                );
            }
        }
        self.history
            .lock()
            .unwrap()
            .retain(|j| !(j.id == original.id && j.status == JobStatus::Cancelled));
        let priority = original.priority as u8;
        let mut pending = self.pending.lock().unwrap();
        let pos = pending.partition_point(|j| (j.priority as u8) >= priority);
        pending.insert(pos, original);
        drop(pending);
        self.notify.notify_one();
    }

    /// Process the next available job using registered handlers.
    /// Returns true if a job was processed.
    pub fn process_one(&self) -> bool {
        self.process_one_with_timeout(Duration::from_millis(WORKER_MIN_POLL_MS))
    }

    fn process_one_with_timeout(&self, timeout: Duration) -> bool {
        let job = match self.dequeue(timeout) {
            Some(j) => j,
            None => return false,
        };

        let heartbeat = self.start_heartbeat(&job.id);
        let handler = {
            let handlers = self.handlers.lock().unwrap();
            handlers.get(&job.name).cloned()
        };

        let result = match handler {
            Some(h) => match h(&job) {
                JobResult::Success => JobResult::Success,
                JobResult::Failure(e) => JobResult::Failure(e),
                JobResult::Retry(reason) => JobResult::Retry(reason),
            },
            None => JobResult::Failure(format!("No handler registered for '{}'", job.name)),
        };
        if let Some((stop, handle)) = heartbeat {
            let _ = stop.send(());
            let _ = handle.join();
        }
        match result {
            JobResult::Success => self.complete(&job.id),
            JobResult::Failure(e) | JobResult::Retry(e) => self.fail(&job.id, &e),
        }

        true
    }

    /// Get job by ID (searches pending, running, history, dead letters).
    pub fn get_job(&self, id: &str) -> Option<Job> {
        if let Some(store) = self.pg_store() {
            return store.load(id).unwrap_or_else(|e| {
                tracing::warn!("[jobs] Postgres get failed for {id}: {e}");
                None
            });
        }
        // Check running first (most common lookup).
        if let Some(j) = self.running.lock().unwrap().get(id) {
            return Some(j.clone());
        }
        // Check pending.
        if let Some(j) = self.pending.lock().unwrap().iter().find(|j| j.id == id) {
            return Some(j.clone());
        }
        // Check history.
        if let Some(j) = self.history.lock().unwrap().iter().find(|j| j.id == id) {
            return Some(j.clone());
        }
        // Check dead letters.
        if let Some(j) = self
            .dead_letters
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.id == id)
        {
            return Some(j.clone());
        }
        None
    }

    /// Get queue statistics.
    pub fn stats(&self) -> QueueStats {
        let handler_names: Vec<String> = self.handlers.lock().unwrap().keys().cloned().collect();
        if let Some(store) = self.pg_store() {
            return store.stats(handler_names.clone()).unwrap_or_else(|e| {
                tracing::warn!("[jobs] Postgres stats failed: {e}");
                QueueStats {
                    pending: 0,
                    running: 0,
                    completed: 0,
                    failed: 0,
                    dead: 0,
                    handlers: handler_names,
                }
            });
        }
        QueueStats {
            pending: self.pending.lock().unwrap().len(),
            running: self.running.lock().unwrap().len(),
            completed: self.completed_count.load(Ordering::Relaxed),
            failed: self.failed_count.load(Ordering::Relaxed),
            dead: self.dead_letters.lock().unwrap().len(),
            handlers: handler_names,
        }
    }

    /// Get pending job count.
    pub fn pending_count(&self) -> usize {
        if self.is_distributed() {
            return self.stats().pending;
        }
        self.pending.lock().unwrap().len()
    }

    /// Get running job count.
    pub fn running_count(&self) -> usize {
        if self.is_distributed() {
            return self.stats().running;
        }
        self.running.lock().unwrap().len()
    }

    /// Get dead letter queue contents.
    pub fn dead_letters(&self) -> Vec<Job> {
        if let Some(store) = self.pg_store() {
            return store.list(Some("dead"), None, 1000).unwrap_or_else(|e| {
                tracing::warn!("[jobs] Postgres dead-letter list failed: {e}");
                Vec::new()
            });
        }
        self.dead_letters.lock().unwrap().iter().cloned().collect()
    }

    /// Retry a dead letter by moving it back to pending.
    pub fn retry_dead(&self, job_id: &str) -> bool {
        if let Some(store) = self.pg_store() {
            return store.retry_dead(job_id).unwrap_or_else(|e| {
                tracing::warn!("[jobs] Postgres dead-letter retry failed for {job_id}: {e}");
                false
            });
        }
        let mut dead = self.dead_letters.lock().unwrap();
        let pos = dead.iter().position(|j| j.id == job_id);
        if let Some(idx) = pos {
            let mut job = dead.remove(idx).unwrap();
            job.status = JobStatus::Pending;
            job.retry_count = 0;
            job.error = None;
            job.started_at = None;
            job.completed_at = None;
            // Clear the back-off stamp: a manual revive means "run it now",
            // not "wait out the schedule the dead job earned".
            job.ready_at = 0;

            let priority = job.priority as u8;
            let mut pending = self.pending.lock().unwrap();
            let insert_pos = pending.partition_point(|j| (j.priority as u8) >= priority);
            pending.insert(insert_pos, job);
            drop(pending);
            drop(dead);
            self.notify.notify_one();
            true
        } else {
            false
        }
    }

    /// Get recent job history.
    pub fn recent_history(&self, limit: usize) -> Vec<Job> {
        let history = self.history.lock().unwrap();
        history.iter().rev().take(limit).cloned().collect()
    }

    /// List pending jobs with optional status/queue filters.
    pub fn list_jobs(&self, status: Option<&str>, queue: Option<&str>, limit: usize) -> Vec<Job> {
        if let Some(store) = self.pg_store() {
            return store.list(status, queue, limit).unwrap_or_else(|e| {
                tracing::warn!("[jobs] Postgres list failed: {e}");
                Vec::new()
            });
        }
        let mut result = Vec::new();

        // Gather from all collections.
        let pending = self.pending.lock().unwrap();
        let running = self.running.lock().unwrap();
        let history = self.history.lock().unwrap();

        let all_jobs = pending.iter().chain(running.values()).chain(history.iter());

        for job in all_jobs {
            if let Some(s) = status {
                if job.status.as_str() != s {
                    continue;
                }
            }
            if let Some(q) = queue {
                if job.queue != q {
                    continue;
                }
            }
            result.push(job.clone());
            if result.len() >= limit {
                break;
            }
        }

        result
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn push_history(&self, job: Job) {
        let mut history = self.history.lock().unwrap();
        history.push_back(job);
        while history.len() > self.max_history {
            history.pop_front();
        }
    }

    fn start_heartbeat(
        &self,
        job_id: &str,
    ) -> Option<(std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>)> {
        let store = self.pg_store()?;
        let token = self.lease_tokens.lock().unwrap().get(job_id).cloned()?;
        let id = job_id.to_string();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("pylon-job-heartbeat".into())
            .spawn(move || loop {
                match stop_rx.recv_timeout(Duration::from_secs(DISTRIBUTED_HEARTBEAT_SECS)) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        match store.heartbeat(&id, &token, DISTRIBUTED_LEASE_SECS) {
                            Ok(true) => {}
                            Ok(false) => {
                                tracing::warn!("[jobs] lease lost while {id} was running");
                                break;
                            }
                            Err(e) => tracing::warn!("[jobs] heartbeat failed for {id}: {e}"),
                        }
                    }
                }
            })
            .ok()?;
        Some((stop_tx, handle))
    }

    fn wait_for_work(&self, timeout: Duration) {
        let pending = self.pending.lock().unwrap();
        let _ = self.notify.wait_timeout(pending, timeout);
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Restore pending/running/retrying jobs AND dead-letter jobs from a
    /// persistent store.
    ///
    /// Jobs that were `Running` at the time of the crash are reset to
    /// `Pending` so they will be re-processed. Dead-letter rows are
    /// re-populated into the in-memory `dead_letters` queue so
    /// `/api/jobs/dead` keeps surfacing them after a restart — without
    /// this the rows persisted on disk but the API returned `[]` until
    /// a new dead-letter happened, hiding the very rows operators need
    /// to triage.
    ///
    /// Returns the number of jobs restored across both pending + dead.
    ///
    /// Call this once at startup, before workers begin processing.
    pub fn restore_from(&self, store: &crate::job_store::JobStore) -> usize {
        let jobs = match store.load_pending() {
            Ok(j) => j,
            Err(_) => Vec::new(),
        };
        let dead = match store.load_dead() {
            Ok(j) => j,
            Err(_) => Vec::new(),
        };

        let mut pending = self.pending.lock().unwrap();
        let pending_count = jobs.len();

        for mut job in jobs {
            // Jobs that were mid-flight when the server died should be
            // treated as pending so they get picked up again.
            if job.status == JobStatus::Running {
                job.status = JobStatus::Pending;
                job.started_at = None;
            }
            if job.status == JobStatus::Retrying {
                job.status = JobStatus::Pending;
            }

            // Insert in priority order.
            let priority = job.priority as u8;
            let pos = pending.partition_point(|j| (j.priority as u8) >= priority);
            pending.insert(pos, job);
        }

        // Re-populate dead-letter queue. `load_dead` returns rows in
        // completed_at DESC; reverse so push_back preserves insertion
        // order semantics (oldest first, newest at the back — matches
        // the live `fail()` path).
        let dead_count = dead.len();
        {
            let mut dead_letters = self.dead_letters.lock().unwrap();
            for job in dead.into_iter().rev() {
                dead_letters.push_back(job);
            }
        }

        pending_count + dead_count
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

/// A worker that continuously processes jobs from the queue.
pub struct Worker {
    queue: Arc<JobQueue>,
    #[allow(dead_code)]
    name: String,
    running: Arc<AtomicBool>,
}

impl Worker {
    pub fn new(queue: Arc<JobQueue>, name: &str) -> Self {
        Self {
            queue,
            name: name.to_string(),
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Start the worker in a background thread. Returns a handle to stop it.
    pub fn start(self) -> WorkerHandle {
        let running = Arc::clone(&self.running);
        let handle = std::thread::spawn(move || {
            let mut poll_ms = WORKER_MIN_POLL_MS;
            while self.running.load(Ordering::Relaxed) {
                if self
                    .queue
                    .process_one_with_timeout(Duration::from_millis(poll_ms))
                {
                    poll_ms = WORKER_MIN_POLL_MS;
                } else {
                    poll_ms = poll_ms.saturating_mul(2).min(WORKER_MAX_POLL_MS);
                }
            }
        });
        WorkerHandle {
            running,
            handle: Some(handle),
        }
    }
}

/// Handle returned by `Worker::start()` to stop the background thread.
pub struct WorkerHandle {
    running: Arc<AtomicBool>,
    #[allow(dead_code)]
    handle: Option<std::thread::JoinHandle<()>>,
}

impl WorkerHandle {
    /// Signal the worker to stop after its current iteration.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_iso() -> String {
    format!("{}Z", now_secs())
}

/// Ceiling on the retry back-off, whatever the attempt count. Five minutes
/// keeps a long-failing job from parking itself for hours while still
/// giving a flapping dependency real room to recover.
const MAX_RETRY_BACKOFF_SECS: u64 = 300;

/// A live worker renews this lease every ten seconds. A failed machine's job
/// becomes claimable by another replica within thirty seconds.
const DISTRIBUTED_LEASE_SECS: u64 = 30;
const DISTRIBUTED_HEARTBEAT_SECS: u64 = 10;
const WORKER_MIN_POLL_MS: u64 = 100;
const WORKER_MAX_POLL_MS: u64 = 2_000;

/// base × 2^(attempt−1), capped. `attempt` is the retry_count AFTER the
/// increment (first retry = 1). A base of 0 disables back-off entirely.
fn retry_backoff_secs(base: u64, attempt: u32) -> u64 {
    if base == 0 || attempt == 0 {
        return 0;
    }
    // Shift capped well below u64 range; the MAX cap makes larger shifts moot.
    let shift = (attempt - 1).min(16);
    base.saturating_mul(1u64 << shift)
        .min(MAX_RETRY_BACKOFF_SECS)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// True when the job may run: past its retry back-off (`ready_at`,
/// absolute epoch seconds — takes precedence when set), else past
/// `created_at + delay_secs`. Jobs with neither are always ready.
fn is_ready(job: &Job, now: u64) -> bool {
    if job.ready_at > 0 {
        return now >= job.ready_at;
    }
    if job.delay_secs == 0 {
        return true;
    }
    let created = job
        .created_at
        .trim_end_matches('Z')
        .parse::<u64>()
        .unwrap_or(0);
    now >= created.saturating_add(job.delay_secs)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_completed_jobs_delegates_and_noops_without_store() {
        let q = JobQueue::new(10);
        // No store attached → safe no-op (never panics).
        assert_eq!(q.cleanup_completed_jobs(0), 0);

        let store = std::sync::Arc::new(crate::job_store::JobStore::in_memory().unwrap());
        let job = Job {
            id: "old".to_string(),
            name: "pylon.cache.cleanup".to_string(),
            payload: serde_json::json!({}),
            priority: Priority::Normal,
            status: JobStatus::Completed,
            max_retries: 3,
            retry_count: 0,
            queue: "default".to_string(),
            delay_secs: 0,
            ready_at: 0,
            error: None,
            created_at: "1000Z".to_string(),
            started_at: None,
            completed_at: Some("1000Z".to_string()), // ancient → prunable
            auth: None,
        };
        store.save(&job).unwrap();
        q.attach_store(std::sync::Arc::clone(&store));

        // max_age 0 → prune everything completed before now.
        assert_eq!(q.cleanup_completed_jobs(0), 1);
        assert_eq!(store.count_by_status("completed"), 0);
    }

    #[test]
    fn enqueue_and_dequeue() {
        let q = JobQueue::new(100);
        let id = q.enqueue("test_job", serde_json::json!({"x": 1}));
        assert!(id.starts_with("job_"));
        assert_eq!(q.pending_count(), 1);

        let job = q.dequeue(Duration::from_millis(10)).unwrap();
        assert_eq!(job.name, "test_job");
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(q.pending_count(), 0);
        assert_eq!(q.running_count(), 1);
    }

    #[test]
    fn try_enqueue_with_auth_round_trips_auth_on_dequeue() {
        // Regression: scheduled jobs previously ran with anonymous auth.
        // The 0.3.76 fix routes the scheduling caller's identity through
        // Job.auth so the handler sees the same `user_id`/`is_admin`/
        // `tenant_id` they would have seen on a direct call.
        let q = JobQueue::new(100);
        let auth = JobAuth {
            user_id: Some("u-alice".into()),
            is_admin: true,
            tenant_id: Some("org_acme".into()),
            is_guest: false,
        };
        let id = q
            .try_enqueue_with_auth(
                "provisionMachine",
                serde_json::json!({"projectId": "p_1"}),
                Priority::Normal,
                0,
                3,
                "functions",
                Some(auth.clone()),
            )
            .unwrap();
        assert!(id.starts_with("job_"));

        let job = q.dequeue(Duration::from_millis(10)).unwrap();
        let job_auth = job.auth.expect("auth should round-trip");
        assert_eq!(job_auth.user_id.as_deref(), Some("u-alice"));
        assert!(job_auth.is_admin);
        assert_eq!(job_auth.tenant_id.as_deref(), Some("org_acme"));
    }

    #[test]
    fn try_enqueue_with_options_leaves_auth_none() {
        // Callers using the original options API (framework cron jobs,
        // dead-letter retry, etc) must keep their pre-0.3.76 behavior
        // of dispatching with no auth context.
        let q = JobQueue::new(100);
        q.try_enqueue_with_options(
            "pylon.cache.cleanup",
            serde_json::json!({}),
            Priority::Normal,
            0,
            3,
            "default",
        )
        .unwrap();
        let job = q.dequeue(Duration::from_millis(10)).unwrap();
        assert!(job.auth.is_none(), "auth should default to None");
    }

    #[test]
    fn dequeue_returns_none_on_empty() {
        let q = JobQueue::new(100);
        assert!(q.dequeue(Duration::from_millis(10)).is_none());
    }

    #[test]
    fn priority_ordering() {
        let q = JobQueue::new(100);
        q.enqueue_with_options("low", serde_json::json!({}), Priority::Low, 0, 0, "default");
        q.enqueue_with_options(
            "high",
            serde_json::json!({}),
            Priority::High,
            0,
            0,
            "default",
        );
        q.enqueue_with_options(
            "normal",
            serde_json::json!({}),
            Priority::Normal,
            0,
            0,
            "default",
        );
        q.enqueue_with_options(
            "critical",
            serde_json::json!({}),
            Priority::Critical,
            0,
            0,
            "default",
        );

        let j1 = q.dequeue(Duration::from_millis(10)).unwrap();
        let j2 = q.dequeue(Duration::from_millis(10)).unwrap();
        let j3 = q.dequeue(Duration::from_millis(10)).unwrap();
        let j4 = q.dequeue(Duration::from_millis(10)).unwrap();

        assert_eq!(j1.name, "critical");
        assert_eq!(j2.name, "high");
        assert_eq!(j3.name, "normal");
        assert_eq!(j4.name, "low");
    }

    #[test]
    #[ignore = "release-mode performance benchmark"]
    fn benchmark_priority_insertion() {
        use std::hint::black_box;
        let q = JobQueue::new(100);
        let jobs: Vec<_> = (0..10_000)
            .map(|_| {
                q.new_job(
                    "work",
                    serde_json::json!({}),
                    Priority::Normal,
                    0,
                    0,
                    "default",
                    None,
                )
            })
            .collect();
        for binary in [false, true] {
            let mut samples = Vec::new();
            for _ in 0..7 {
                let inputs = jobs.clone();
                let mut pending: VecDeque<Job> = VecDeque::with_capacity(inputs.len());
                let start = std::time::Instant::now();
                for job in inputs {
                    let priority = black_box(job.priority as u8);
                    let index = if binary {
                        pending.partition_point(|j| (j.priority as u8) >= priority)
                    } else {
                        pending
                            .iter()
                            .position(|j| (j.priority as u8) < priority)
                            .unwrap_or(pending.len())
                    };
                    pending.insert(index, job);
                }
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(black_box(pending.len()), 10_000);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "10000 equal-priority job insertions, binary={binary}: {:.3} ms median",
                samples[3]
            );
        }
    }

    #[test]
    fn priority_insertion_preserves_fifo_after_retries_and_restore() {
        let q = JobQueue::new(100);
        q.set_retry_backoff_base_secs(0);
        let mut expected: Vec<(String, Priority)> = Vec::new();
        let insert_expected =
            |expected: &mut Vec<(String, Priority)>, id: String, priority: Priority| {
                let index = expected
                    .iter()
                    .position(|(_, p)| (*p as u8) < (priority as u8))
                    .unwrap_or(expected.len());
                expected.insert(index, (id, priority));
            };
        let assert_order = |q: &JobQueue, expected: &[(String, Priority)]| {
            let pending = q.pending.lock().unwrap();
            assert_eq!(
                pending
                    .iter()
                    .map(|job| (&job.id, job.priority))
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|(id, priority)| (id, *priority))
                    .collect::<Vec<_>>()
            );
        };
        for i in 0..128 {
            let priority = [
                Priority::Low,
                Priority::High,
                Priority::Normal,
                Priority::Critical,
            ][i % 4];
            let id = q.enqueue_with_options(
                "work",
                serde_json::json!({"i":i}),
                priority,
                0,
                1,
                "default",
            );
            insert_expected(&mut expected, id, priority);
            if i % 7 == 6 {
                let job = q.dequeue(Duration::ZERO).unwrap();
                assert_eq!(job.id, expected.remove(0).0);
                q.complete(&job.id);
            }
            assert_order(&q, &expected);
        }
        let job = q.dequeue(Duration::ZERO).unwrap();
        assert_eq!(job.id, expected.remove(0).0);
        q.fail(&job.id, "retry");
        insert_expected(&mut expected, job.id.clone(), job.priority);
        assert_order(&q, &expected);
        let cancelled = q.cancel_pending_local(&job.id).unwrap().unwrap();
        expected.retain(|(id, _)| id != &job.id);
        q.restore_cancelled(cancelled);
        insert_expected(&mut expected, job.id, job.priority);
        assert_order(&q, &expected);

        let dead_id = q.enqueue_with_options(
            "dead",
            serde_json::json!({}),
            Priority::Critical,
            0,
            0,
            "default",
        );
        insert_expected(&mut expected, dead_id.clone(), Priority::Critical);
        while let Some(job) = q.dequeue(Duration::ZERO) {
            assert_eq!(job.id, expected.remove(0).0);
            if job.id == dead_id {
                q.fail(&job.id, "dead");
                break;
            }
            q.complete(&job.id);
        }
        assert!(q.retry_dead(&dead_id));
        insert_expected(&mut expected, dead_id, Priority::Critical);
        assert_order(&q, &expected);

        let store = crate::job_store::JobStore::in_memory().unwrap();
        let mut restored = q.pending.lock().unwrap().front().unwrap().clone();
        restored.id = "restored-priority-job".into();
        restored.priority = Priority::Normal;
        store.save(&restored).unwrap();
        assert_eq!(q.restore_from(&store), 1);
        insert_expected(&mut expected, restored.id, restored.priority);
        assert_order(&q, &expected);
    }

    #[test]
    fn complete_moves_to_history() {
        let q = JobQueue::new(100);
        let id = q.enqueue("test", serde_json::json!({}));
        let _job = q.dequeue(Duration::from_millis(10)).unwrap();
        q.complete(&id);

        assert_eq!(q.running_count(), 0);
        let job = q.get_job(&id).unwrap();
        assert_eq!(job.status, JobStatus::Completed);
    }

    #[test]
    fn fail_retries_when_under_max() {
        let q = JobQueue::new(100);
        // The default back-off is one second, which turns the "not
        // dequeuable" assertion at the end of this test into a race with the
        // wall clock: `fail` sets ready_at to now+1, and if the second ticks
        // over during the 10ms dequeue wait the job IS ready and the test
        // fails. It surfaced on a Windows runner first, but nothing about it
        // is platform-specific — it is roughly a 1-in-100 coin flip anywhere.
        //
        // Keep the property that matters (a non-zero default, which is what
        // stops a deterministic failure burning every retry in milliseconds)
        // and then pin a back-off no dequeue in this test can outlast. The
        // schedule itself is covered by
        // `retry_backoff_schedule_doubles_and_caps`.
        assert!(
            q.retry_backoff_base_secs.load(Ordering::Relaxed) > 0,
            "the default retry back-off must not be zero"
        );
        q.set_retry_backoff_base_secs(60);
        let id = q.enqueue_with_options(
            "test",
            serde_json::json!({}),
            Priority::Normal,
            0,
            2,
            "default",
        );

        // First attempt -- fail.
        let _job = q.dequeue(Duration::from_millis(10)).unwrap();
        q.fail(&id, "oops");

        // Should be back in pending with retry_count=1.
        let job = q.get_job(&id).unwrap();
        assert_eq!(job.retry_count, 1);
        assert_eq!(job.status, JobStatus::Retrying);
        assert_eq!(q.pending_count(), 1);

        // The retry earned a back-off: it is NOT immediately dequeuable
        // (this is the bug where a deterministic failure burned every
        // retry in milliseconds and dead-lettered instantly).
        assert!(job.ready_at > now_secs());
        assert!(q.dequeue(Duration::from_millis(10)).is_none());
    }

    #[test]
    fn retry_backoff_schedule_doubles_and_caps() {
        // base × 2^(attempt−1), capped at MAX_RETRY_BACKOFF_SECS.
        assert_eq!(retry_backoff_secs(1, 1), 1);
        assert_eq!(retry_backoff_secs(1, 2), 2);
        assert_eq!(retry_backoff_secs(1, 3), 4);
        assert_eq!(retry_backoff_secs(1, 5), 16);
        assert_eq!(retry_backoff_secs(1, 9), 256);
        assert_eq!(retry_backoff_secs(1, 10), MAX_RETRY_BACKOFF_SECS);
        assert_eq!(retry_backoff_secs(1, 63), MAX_RETRY_BACKOFF_SECS);
        // A base of 0 disables the delay entirely (test hook).
        assert_eq!(retry_backoff_secs(0, 5), 0);
        // Attempt 0 never happens (fail() increments first) but must not
        // underflow the shift.
        assert_eq!(retry_backoff_secs(1, 0), 0);
    }

    #[test]
    fn ready_at_gates_readiness_and_zero_falls_back() {
        let now = now_secs();
        let mut job = Job {
            id: "j".into(),
            name: "t".into(),
            payload: serde_json::json!({}),
            priority: Priority::Normal,
            status: JobStatus::Retrying,
            max_retries: 3,
            retry_count: 1,
            created_at: now_iso(),
            started_at: None,
            completed_at: None,
            error: None,
            delay_secs: 0,
            ready_at: now + 30,
            queue: "default".into(),
            auth: None,
        };
        assert!(!is_ready(&job, now));
        assert!(is_ready(&job, now + 30));
        // ready_at = 0 → legacy created_at + delay_secs readiness.
        job.ready_at = 0;
        assert!(is_ready(&job, now));
    }

    #[test]
    fn retry_dead_clears_backoff_stamp() {
        let q = JobQueue::new(100);
        // Base 0 still stamps ready_at (= now) on the retry, so the dead
        // job carries a stale stamp for retry_dead to clear.
        q.set_retry_backoff_base_secs(0);
        let id = q.enqueue_with_options(
            "test",
            serde_json::json!({}),
            Priority::Normal,
            0,
            1,
            "default",
        );
        let _job = q.dequeue(Duration::from_millis(10)).unwrap();
        q.fail(&id, "fail 1");
        let _job = q.dequeue(Duration::from_millis(10)).unwrap();
        q.fail(&id, "fail 2");
        let dead = q.dead_letters();
        assert_eq!(dead.len(), 1);
        assert!(dead[0].ready_at > 0);

        assert!(q.retry_dead(&id));
        let job = q.get_job(&id).unwrap();
        assert_eq!(job.ready_at, 0);
    }

    #[test]
    fn fail_moves_to_dead_after_max_retries() {
        let q = JobQueue::new(100);
        // Zero back-off keeps the fail→dequeue→fail loop synchronous.
        q.set_retry_backoff_base_secs(0);
        let id = q.enqueue_with_options(
            "test",
            serde_json::json!({}),
            Priority::Normal,
            0,
            1,
            "default",
        );

        // Attempt 1 -- fail.
        let _job = q.dequeue(Duration::from_millis(10)).unwrap();
        q.fail(&id, "fail 1");

        // Attempt 2 (retry_count=1 == max_retries=1) -- fail again.
        let _job = q.dequeue(Duration::from_millis(10)).unwrap();
        q.fail(&id, "fail 2");

        // Should be dead now.
        let dead = q.dead_letters();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].id, id);
        assert_eq!(dead[0].status, JobStatus::Dead);
    }

    #[test]
    fn retry_dead_letter() {
        let q = JobQueue::new(100);
        let id = q.enqueue_with_options(
            "test",
            serde_json::json!({}),
            Priority::Normal,
            0,
            0,
            "default",
        );

        let _job = q.dequeue(Duration::from_millis(10)).unwrap();
        q.fail(&id, "dead");
        assert_eq!(q.dead_letters().len(), 1);

        assert!(q.retry_dead(&id));
        assert_eq!(q.dead_letters().len(), 0);
        assert_eq!(q.pending_count(), 1);

        let job = q.get_job(&id).unwrap();
        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.retry_count, 0);
    }

    #[test]
    fn retry_dead_returns_false_for_unknown() {
        let q = JobQueue::new(100);
        assert!(!q.retry_dead("nonexistent"));
    }

    #[test]
    fn get_job_searches_all_collections() {
        let q = JobQueue::new(100);
        let id1 = q.enqueue("pending_job", serde_json::json!({}));
        assert!(q.get_job(&id1).is_some());

        let id2 = q.enqueue("running_job", serde_json::json!({}));
        let _job = q.dequeue(Duration::from_millis(10)).unwrap(); // dequeues id1 (earlier)
        let _job = q.dequeue(Duration::from_millis(10)).unwrap(); // dequeues id2
        assert!(q.get_job(&id2).is_some());

        q.complete(&id1);
        let found = q.get_job(&id1).unwrap();
        assert_eq!(found.status, JobStatus::Completed);
    }

    #[test]
    fn dequeue_from_specific_queue() {
        let q = JobQueue::new(100);
        q.enqueue_with_options("a", serde_json::json!({}), Priority::High, 0, 0, "alpha");
        q.enqueue_with_options("b", serde_json::json!({}), Priority::Critical, 0, 0, "beta");

        let job = q.dequeue_from("beta", Duration::from_millis(10)).unwrap();
        assert_eq!(job.name, "b");
        assert_eq!(job.queue, "beta");
    }

    #[test]
    fn process_one_with_handler() {
        let q = Arc::new(JobQueue::new(100));
        q.register("echo", Arc::new(|_job| JobResult::Success));
        q.enqueue("echo", serde_json::json!({"msg": "hello"}));
        assert!(q.process_one());

        let stats = q.stats();
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.pending, 0);
    }

    #[test]
    fn process_one_without_handler_fails() {
        let q = Arc::new(JobQueue::new(100));
        q.enqueue_with_options(
            "unhandled",
            serde_json::json!({}),
            Priority::Normal,
            0,
            0,
            "default",
        );
        q.process_one();

        // Should be in dead letters since max_retries=0.
        assert_eq!(q.dead_letters().len(), 1);
    }

    #[test]
    fn stats_reports_handler_names() {
        let q = JobQueue::new(100);
        q.register("alpha", Arc::new(|_| JobResult::Success));
        q.register("beta", Arc::new(|_| JobResult::Success));

        let stats = q.stats();
        assert!(stats.handlers.contains(&"alpha".to_string()));
        assert!(stats.handlers.contains(&"beta".to_string()));
    }

    #[test]
    fn history_is_bounded() {
        let q = JobQueue::new(3);
        for i in 0..5 {
            let id = q.enqueue(&format!("job_{i}"), serde_json::json!({}));
            let _job = q.dequeue(Duration::from_millis(10)).unwrap();
            q.complete(&id);
        }
        let history = q.recent_history(10);
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn list_jobs_with_filters() {
        let q = JobQueue::new(100);
        q.enqueue_with_options("a", serde_json::json!({}), Priority::Normal, 0, 0, "emails");
        q.enqueue_with_options(
            "b",
            serde_json::json!({}),
            Priority::Normal,
            0,
            0,
            "default",
        );
        q.enqueue_with_options("c", serde_json::json!({}), Priority::Normal, 0, 0, "emails");

        let email_jobs = q.list_jobs(None, Some("emails"), 50);
        assert_eq!(email_jobs.len(), 2);

        let pending_jobs = q.list_jobs(Some("pending"), None, 50);
        assert_eq!(pending_jobs.len(), 3);
    }

    #[test]
    fn worker_processes_jobs() {
        let q = Arc::new(JobQueue::new(100));
        q.register("add", Arc::new(|_job| JobResult::Success));
        q.enqueue("add", serde_json::json!({"a": 1, "b": 2}));

        let worker = Worker::new(Arc::clone(&q), "test-worker");
        let handle = worker.start();

        // Give the worker time to pick up the job.
        std::thread::sleep(Duration::from_millis(200));
        handle.stop();

        assert_eq!(q.stats().completed, 1);
    }

    #[test]
    fn priority_from_str_loose() {
        assert_eq!(Priority::from_str_loose("low"), Priority::Low);
        assert_eq!(Priority::from_str_loose("HIGH"), Priority::High);
        assert_eq!(Priority::from_str_loose("critical"), Priority::Critical);
        assert_eq!(Priority::from_str_loose("unknown"), Priority::Normal);
    }

    #[test]
    fn restore_from_store() {
        let store = crate::job_store::JobStore::in_memory().unwrap();

        // Save some jobs to the store with different statuses.
        let pending_job = Job {
            id: "job_100".into(),
            name: "email".into(),
            payload: serde_json::json!({"to": "alice"}),
            priority: Priority::High,
            status: JobStatus::Pending,
            max_retries: 3,
            retry_count: 0,
            queue: "default".into(),
            delay_secs: 0,
            ready_at: 0,
            error: None,
            created_at: "1000Z".into(),
            started_at: None,
            completed_at: None,
            auth: None,
        };
        let running_job = Job {
            id: "job_200".into(),
            name: "process".into(),
            payload: serde_json::json!({}),
            priority: Priority::Normal,
            status: JobStatus::Running,
            max_retries: 2,
            retry_count: 1,
            queue: "default".into(),
            delay_secs: 0,
            ready_at: 0,
            error: None,
            created_at: "2000Z".into(),
            started_at: Some("2001Z".into()),
            completed_at: None,
            auth: None,
        };

        store.save(&pending_job).unwrap();
        store.save(&running_job).unwrap();

        let q = JobQueue::new(100);
        let restored = q.restore_from(&store);
        assert_eq!(restored, 2);
        assert_eq!(q.pending_count(), 2);

        // Running job should have been reset to Pending.
        let job = q.get_job("job_200").unwrap();
        assert_eq!(job.status, JobStatus::Pending);
        assert!(job.started_at.is_none());

        // A new id never repeats a restored one.
        let new_id = q.enqueue("new", serde_json::json!({}));
        assert!(new_id != "job_100" && new_id != "job_200");
        assert_eq!(q.pending_count(), 3);
    }

    #[test]
    fn restore_from_store_repopulates_dead_letters() {
        // Regression: dead-letter rows persisted on disk used to be
        // lost from `/api/jobs/dead` after a restart because
        // `restore_from` only loaded pending/running/retrying. Now it
        // re-populates the in-memory dead-letter queue too, so
        // operators don't lose visibility into failed jobs at the
        // first deploy.
        let store = crate::job_store::JobStore::in_memory().unwrap();

        let dead_job = Job {
            id: "job_999".into(),
            name: "provisionMachine".into(),
            payload: serde_json::json!({"projectId": "p_1"}),
            priority: Priority::Normal,
            status: JobStatus::Dead,
            max_retries: 3,
            retry_count: 3,
            queue: "functions".into(),
            delay_secs: 0,
            ready_at: 0,
            error: Some("UNAUTHENTICATED: log in first".into()),
            created_at: "5000Z".into(),
            started_at: Some("5001Z".into()),
            completed_at: Some("5050Z".into()),
            auth: None,
        };
        store.save(&dead_job).unwrap();

        let q = JobQueue::new(100);
        let restored = q.restore_from(&store);
        assert_eq!(restored, 1);
        assert_eq!(q.pending_count(), 0);

        let dead = q.dead_letters();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].id, "job_999");
        assert_eq!(dead[0].name, "provisionMachine");
        assert_eq!(
            dead[0].error.as_deref(),
            Some("UNAUTHENTICATED: log in first")
        );

        // A new id must not repeat the dead job's id, or INSERT OR REPLACE
        // overwrites the dead row.
        let new_id = q.enqueue("fresh", serde_json::json!({}));
        assert_ne!(new_id, "job_999");
    }

    // -----------------------------------------------------------------------
    // Cancellation
    // -----------------------------------------------------------------------

    fn delayed(q: &JobQueue, delay_secs: u64) -> String {
        q.try_enqueue_with_options(
            "reminder",
            serde_json::json!({"leadId": "l_1"}),
            Priority::Normal,
            delay_secs,
            3,
            "functions",
        )
        .unwrap()
    }

    #[test]
    fn job_ids_do_not_repeat_across_processes() {
        // A restarted process must never hand out an id that an app still
        // holds from the previous process, or `cancel(oldId)` would cancel
        // someone else's job.
        let first = JobQueue::new(10);
        let second = JobQueue::new(10);
        let a = first.enqueue("x", serde_json::json!({}));
        let b = second.enqueue("x", serde_json::json!({}));
        assert_ne!(a, b);
    }

    #[test]
    fn cancel_pending_delayed_job_removes_it_and_marks_it_cancelled() {
        let q = JobQueue::new(100);
        let id = delayed(&q, 3600);

        assert!(q.cancel_pending(&id).unwrap());

        assert_eq!(q.pending_count(), 0);
        assert!(q.dequeue(Duration::from_millis(10)).is_none());
        let job = q.get_job(&id).unwrap();
        assert_eq!(job.status, JobStatus::Cancelled);
        assert!(job.completed_at.is_some());
        assert_eq!(q.list_jobs(Some("cancelled"), None, 10).len(), 1);
    }

    #[test]
    fn cancelled_ready_job_never_reaches_its_handler() {
        let q = JobQueue::new(100);
        let ran = Arc::new(AtomicU64::new(0));
        let ran_in_handler = Arc::clone(&ran);
        q.register(
            "reminder",
            Arc::new(move |_| {
                ran_in_handler.fetch_add(1, Ordering::SeqCst);
                JobResult::Success
            }),
        );
        let id = delayed(&q, 0);
        assert!(q.cancel_pending(&id).unwrap());
        assert!(!q.process_one());
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cancel_running_job_returns_false_and_leaves_it_running() {
        let q = JobQueue::new(100);
        let id = q.enqueue("test", serde_json::json!({}));
        let _job = q.dequeue(Duration::from_millis(10)).unwrap();

        assert!(!q.cancel_pending(&id).unwrap());

        let job = q.get_job(&id).unwrap();
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(q.running_count(), 1);
    }

    #[test]
    fn cancel_after_the_job_ran_returns_false() {
        let q = JobQueue::new(100);
        q.register("reminder", Arc::new(|_| JobResult::Success));
        let id = delayed(&q, 0);
        assert!(q.process_one());

        assert!(!q.cancel_pending(&id).unwrap());
        assert_eq!(q.get_job(&id).unwrap().status, JobStatus::Completed);
    }

    #[test]
    fn cancel_twice_returns_true_then_false() {
        let q = JobQueue::new(100);
        let id = delayed(&q, 3600);
        assert!(q.cancel_pending(&id).unwrap());
        assert!(!q.cancel_pending(&id).unwrap());
        assert_eq!(q.get_job(&id).unwrap().status, JobStatus::Cancelled);
    }

    #[test]
    fn cancel_unknown_job_returns_false() {
        let q = JobQueue::new(100);
        assert!(!q.cancel_pending("job_missing").unwrap());
        assert!(!q.cancel_pending("").unwrap());
    }

    #[test]
    fn cancel_a_job_waiting_to_retry() {
        let q = JobQueue::new(100);
        let id = q.enqueue("flaky", serde_json::json!({}));
        let _ = q.dequeue(Duration::from_millis(10)).unwrap();
        q.fail(&id, "boom");
        assert_eq!(q.get_job(&id).unwrap().status, JobStatus::Retrying);

        assert!(q.cancel_pending(&id).unwrap());
        assert_eq!(q.pending_count(), 0);
        assert_eq!(q.get_job(&id).unwrap().status, JobStatus::Cancelled);
    }

    #[test]
    fn cancelled_job_is_not_restored_after_restart() {
        let store = Arc::new(crate::job_store::JobStore::in_memory().unwrap());
        let q = JobQueue::new(100);
        q.attach_store(Arc::clone(&store));
        let cancelled = delayed(&q, 3600);
        let kept = delayed(&q, 3600);
        assert!(q.cancel_pending(&cancelled).unwrap());

        assert_eq!(
            store.load(&cancelled).unwrap().unwrap().status,
            JobStatus::Cancelled
        );

        // A new process restores from the same store.
        let restarted = JobQueue::new(100);
        assert_eq!(restarted.restore_from(&store), 1);
        assert!(restarted.get_job(&cancelled).is_none());
        assert_eq!(restarted.get_job(&kept).unwrap().status, JobStatus::Pending);
        assert!(!restarted.cancel_pending(&cancelled).unwrap());
    }

    #[test]
    fn claim_and_cancel_race_has_exactly_one_winner() {
        // A worker and a cancel race for the same ready job, many times.
        // Each round, exactly one side wins: either the handler runs and
        // cancel reports false, or cancel reports true and the handler
        // never runs.
        for _ in 0..200 {
            let q = Arc::new(JobQueue::new(10));
            let ran = Arc::new(AtomicU64::new(0));
            let ran_in_handler = Arc::clone(&ran);
            q.register(
                "reminder",
                Arc::new(move |_| {
                    ran_in_handler.fetch_add(1, Ordering::SeqCst);
                    JobResult::Success
                }),
            );
            let id = delayed(&q, 0);
            let worker_q = Arc::clone(&q);
            let worker =
                std::thread::spawn(move || worker_q.process_one_with_timeout(Duration::ZERO));
            let cancelled = q.cancel_pending(&id).unwrap();
            worker.join().unwrap();
            // Drain anything the worker missed so a lost job would show.
            while q.process_one_with_timeout(Duration::ZERO) {}
            let runs = ran.load(Ordering::SeqCst);
            assert_eq!(
                runs + u64::from(cancelled),
                1,
                "cancelled={cancelled} runs={runs}"
            );
        }
    }
}
