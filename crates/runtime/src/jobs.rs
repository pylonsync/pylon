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
    /// Queue name (for routing to specific workers).
    pub queue: String,
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
    /// Optional persistent backing store. When set, every state transition is
    /// mirrored to SQLite so jobs survive restart. Failures to persist are
    /// logged but not surfaced — durability is best-effort, never blocking.
    store: Mutex<Option<std::sync::Arc<crate::job_store::JobStore>>>,
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
            store: Mutex::new(None),
        }
    }

    /// Attach a persistent store. After this, every enqueue, state change, and
    /// terminal event is mirrored to the store. Call once at startup.
    pub fn attach_store(&self, store: std::sync::Arc<crate::job_store::JobStore>) {
        *self.store.lock().unwrap() = Some(store);
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
        let id = format!("job_{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let now = now_iso();
        let job = Job {
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
            queue: queue.to_string(),
        };
        self.try_enqueue_job(job)
    }

    fn try_enqueue_job(&self, job: Job) -> Result<String, String> {
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
            let pos = pending
                .iter()
                .position(|j| (j.priority as u8) < (priority as u8))
                .unwrap_or(pending.len());
            pending.insert(pos, job);
        }
        self.notify.notify_one();
        Ok(id)
    }

    /// Dequeue the highest-priority pending job whose `delay_secs` has
    /// elapsed. Blocks up to `timeout` if nothing is ready.
    pub fn dequeue(&self, timeout: Duration) -> Option<Job> {
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
            job.status = JobStatus::Completed;
            job.completed_at = Some(now_iso());
            self.completed_count.fetch_add(1, Ordering::Relaxed);
            self.persist(&job);
            self.push_history(job);
        }
    }

    /// Mark a job as failed. Retries if under max_retries.
    pub fn fail(&self, job_id: &str, error: &str) {
        let job = self.running.lock().unwrap().remove(job_id);
        if let Some(mut job) = job {
            job.error = Some(error.to_string());

            if job.retry_count < job.max_retries {
                // Re-enqueue for retry.
                job.retry_count += 1;
                job.status = JobStatus::Retrying;
                job.started_at = None;
                job.completed_at = None;

                self.persist(&job);
                let mut pending = self.pending.lock().unwrap();
                let priority = job.priority as u8;
                let pos = pending
                    .iter()
                    .position(|j| (j.priority as u8) < priority)
                    .unwrap_or(pending.len());
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

    /// Process the next available job using registered handlers.
    /// Returns true if a job was processed.
    pub fn process_one(&self) -> bool {
        let job = match self.dequeue(Duration::from_millis(100)) {
            Some(j) => j,
            None => return false,
        };

        let handler = {
            let handlers = self.handlers.lock().unwrap();
            handlers.get(&job.name).cloned()
        };

        match handler {
            Some(h) => match h(&job) {
                JobResult::Success => self.complete(&job.id),
                JobResult::Failure(e) => self.fail(&job.id, &e),
                JobResult::Retry(reason) => self.fail(&job.id, &reason),
            },
            None => {
                self.fail(
                    &job.id,
                    &format!("No handler registered for '{}'", job.name),
                );
            }
        }

        true
    }

    /// Get job by ID (searches pending, running, history, dead letters).
    pub fn get_job(&self, id: &str) -> Option<Job> {
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
        self.pending.lock().unwrap().len()
    }

    /// Get running job count.
    pub fn running_count(&self) -> usize {
        self.running.lock().unwrap().len()
    }

    /// Get dead letter queue contents.
    pub fn dead_letters(&self) -> Vec<Job> {
        self.dead_letters.lock().unwrap().iter().cloned().collect()
    }

    /// Retry a dead letter by moving it back to pending.
    pub fn retry_dead(&self, job_id: &str) -> bool {
        let mut dead = self.dead_letters.lock().unwrap();
        let pos = dead.iter().position(|j| j.id == job_id);
        if let Some(idx) = pos {
            let mut job = dead.remove(idx).unwrap();
            job.status = JobStatus::Pending;
            job.retry_count = 0;
            job.error = None;
            job.started_at = None;
            job.completed_at = None;

            let priority = job.priority as u8;
            let mut pending = self.pending.lock().unwrap();
            let insert_pos = pending
                .iter()
                .position(|j| (j.priority as u8) < priority)
                .unwrap_or(pending.len());
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
        let mut result = Vec::new();

        // Gather from all collections.
        let pending = self.pending.lock().unwrap();
        let running = self.running.lock().unwrap();
        let history = self.history.lock().unwrap();

        let all_jobs = pending.iter().chain(running.values()).chain(history.iter());

        for job in all_jobs {
            if let Some(s) = status {
                let job_status = match &job.status {
                    JobStatus::Pending => "pending",
                    JobStatus::Running => "running",
                    JobStatus::Completed => "completed",
                    JobStatus::Failed => "failed",
                    JobStatus::Retrying => "retrying",
                    JobStatus::Dead => "dead",
                };
                if job_status != s {
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

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Restore pending/running/retrying jobs from a persistent store.
    ///
    /// Jobs that were `Running` at the time of the crash are reset to
    /// `Pending` so they will be re-processed. Returns the number of jobs
    /// restored.
    ///
    /// Call this once at startup, before workers begin processing.
    pub fn restore_from(&self, store: &crate::job_store::JobStore) -> usize {
        let jobs = match store.load_pending() {
            Ok(j) => j,
            Err(_) => return 0,
        };

        let mut pending = self.pending.lock().unwrap();
        let count = jobs.len();

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
            let pos = pending
                .iter()
                .position(|j| (j.priority as u8) < priority)
                .unwrap_or(pending.len());
            pending.insert(pos, job);
        }

        // Ensure the ID counter doesn't collide with restored IDs.
        // Parse the numeric suffix from "job_N" and set next_id above the max.
        let max_id = pending
            .iter()
            .filter_map(|j| {
                j.id.strip_prefix("job_")
                    .and_then(|n| n.parse::<u64>().ok())
            })
            .max()
            .unwrap_or(0);
        let current = self.next_id.load(Ordering::Relaxed);
        if max_id >= current {
            self.next_id.store(max_id + 1, Ordering::Relaxed);
        }

        count
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
            while self.running.load(Ordering::Relaxed) {
                self.queue.process_one();
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// True if a job's `delay_secs` has elapsed since `created_at`. Jobs
/// without a delay (`delay_secs == 0`) are always ready.
fn is_ready(job: &Job, now: u64) -> bool {
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
    }

    #[test]
    fn fail_moves_to_dead_after_max_retries() {
        let q = JobQueue::new(100);
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
            error: None,
            created_at: "1000Z".into(),
            started_at: None,
            completed_at: None,
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
            error: None,
            created_at: "2000Z".into(),
            started_at: Some("2001Z".into()),
            completed_at: None,
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

        // ID counter should be past restored IDs.
        let new_id = q.enqueue("new", serde_json::json!({}));
        let num: u64 = new_id.strip_prefix("job_").unwrap().parse().unwrap();
        assert!(num > 200);
    }
}
