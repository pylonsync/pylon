//! SQLite-backed persistent storage for jobs.
//!
//! Persists jobs to a SQLite database so they survive server restarts.
//! The in-memory queue remains the source of truth at runtime; the store
//! is written to on every state change and read from only at startup to
//! restore unfinished work.

use rusqlite::Connection;
use std::sync::Mutex;

use crate::jobs::{Job, JobStatus, Priority};

/// SQLite-backed persistent storage for jobs.
pub struct JobStore {
    conn: Mutex<Connection>,
}

impl JobStore {
    /// Open or create the job store database at `path`.
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("Failed to open job store: {e}"))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    /// Create an in-memory store (useful for tests).
    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory store: {e}"))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                payload TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL DEFAULT 'pending',
                max_retries INTEGER NOT NULL DEFAULT 3,
                retry_count INTEGER NOT NULL DEFAULT 0,
                queue TEXT NOT NULL DEFAULT 'default',
                delay_secs INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                created_at TEXT NOT NULL,
                started_at TEXT,
                completed_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
            CREATE INDEX IF NOT EXISTS idx_jobs_queue ON jobs(queue);
        ",
        )
        .map_err(|e| format!("Schema init failed: {e}"))
    }

    /// Save a job (insert or update).
    pub fn save(&self, job: &Job) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO jobs \
             (id, name, payload, priority, status, max_retries, retry_count, \
              queue, delay_secs, error, created_at, started_at, completed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                job.id,
                job.name,
                job.payload.to_string(),
                priority_to_int(&job.priority),
                status_to_str(&job.status),
                job.max_retries,
                job.retry_count,
                job.queue,
                job.delay_secs,
                job.error,
                job.created_at,
                job.started_at,
                job.completed_at,
            ],
        )
        .map_err(|e| format!("Save failed: {e}"))?;
        Ok(())
    }

    /// Load a job by ID.
    pub fn load(&self, id: &str) -> Result<Option<Job>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, payload, priority, status, max_retries, retry_count, \
                 queue, delay_secs, error, created_at, started_at, completed_at \
                 FROM jobs WHERE id = ?1",
            )
            .map_err(|e| format!("Prepare failed: {e}"))?;

        let result = stmt
            .query_row(rusqlite::params![id], |row| Ok(row_to_job(row)))
            .ok();

        Ok(result)
    }

    /// Load all pending/running/retrying jobs (for recovery after restart).
    ///
    /// Jobs that were `running` at the time of a crash are included so they
    /// can be re-enqueued. The caller is responsible for resetting their
    /// status to `Pending` before re-inserting into the in-memory queue.
    pub fn load_pending(&self) -> Result<Vec<Job>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, payload, priority, status, max_retries, retry_count, \
                 queue, delay_secs, error, created_at, started_at, completed_at \
                 FROM jobs \
                 WHERE status IN ('pending', 'running', 'retrying') \
                 ORDER BY priority DESC, created_at ASC",
            )
            .map_err(|e| format!("Prepare failed: {e}"))?;

        let rows = stmt
            .query_map([], |row| Ok(row_to_job(row)))
            .map_err(|e| format!("Query failed: {e}"))?;

        let mut jobs = Vec::new();
        for row in rows {
            if let Ok(job) = row {
                jobs.push(job);
            }
        }
        Ok(jobs)
    }

    /// Load dead-letter jobs.
    pub fn load_dead(&self) -> Result<Vec<Job>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, payload, priority, status, max_retries, retry_count, \
                 queue, delay_secs, error, created_at, started_at, completed_at \
                 FROM jobs \
                 WHERE status = 'dead' \
                 ORDER BY completed_at DESC",
            )
            .map_err(|e| format!("Prepare failed: {e}"))?;

        let rows = stmt
            .query_map([], |row| Ok(row_to_job(row)))
            .map_err(|e| format!("Query failed: {e}"))?;

        let mut jobs = Vec::new();
        for row in rows {
            if let Ok(job) = row {
                jobs.push(job);
            }
        }
        Ok(jobs)
    }

    /// Count jobs by status.
    pub fn count_by_status(&self, status: &str) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE status = ?1",
            rusqlite::params![status],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    /// Delete old completed/dead jobs older than `max_age_secs`.
    ///
    /// Returns the number of rows deleted.
    pub fn cleanup_completed(&self, max_age_secs: u64) -> usize {
        let conn = self.conn.lock().unwrap();
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(max_age_secs);
        let cutoff_str = format!("{cutoff}Z");

        conn.execute(
            "DELETE FROM jobs WHERE status IN ('completed', 'dead') AND completed_at < ?1",
            rusqlite::params![cutoff_str],
        )
        .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Row mapping helpers
// ---------------------------------------------------------------------------

fn row_to_job(row: &rusqlite::Row<'_>) -> Job {
    Job {
        id: row.get(0).unwrap_or_default(),
        name: row.get(1).unwrap_or_default(),
        payload: serde_json::from_str(&row.get::<_, String>(2).unwrap_or_default())
            .unwrap_or(serde_json::json!({})),
        priority: int_to_priority(row.get(3).unwrap_or(1)),
        status: str_to_status(&row.get::<_, String>(4).unwrap_or_default()),
        max_retries: row.get(5).unwrap_or(3),
        retry_count: row.get(6).unwrap_or(0),
        queue: row.get(7).unwrap_or_default(),
        delay_secs: row.get(8).unwrap_or(0),
        error: row.get(9).ok(),
        created_at: row.get(10).unwrap_or_default(),
        started_at: row.get(11).ok(),
        completed_at: row.get(12).ok(),
    }
}

fn priority_to_int(p: &Priority) -> i32 {
    match p {
        Priority::Low => 0,
        Priority::Normal => 1,
        Priority::High => 2,
        Priority::Critical => 3,
    }
}

fn int_to_priority(n: i32) -> Priority {
    match n {
        0 => Priority::Low,
        2 => Priority::High,
        3 => Priority::Critical,
        _ => Priority::Normal,
    }
}

fn status_to_str(s: &JobStatus) -> &'static str {
    match s {
        JobStatus::Pending => "pending",
        JobStatus::Running => "running",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::Retrying => "retrying",
        JobStatus::Dead => "dead",
    }
}

fn str_to_status(s: &str) -> JobStatus {
    match s {
        "pending" => JobStatus::Pending,
        "running" => JobStatus::Running,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        "retrying" => JobStatus::Retrying,
        "dead" => JobStatus::Dead,
        _ => JobStatus::Pending,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_job(id: &str, status: JobStatus) -> Job {
        Job {
            id: id.to_string(),
            name: "test_job".to_string(),
            payload: serde_json::json!({"key": "value"}),
            priority: Priority::Normal,
            status,
            max_retries: 3,
            retry_count: 0,
            queue: "default".to_string(),
            delay_secs: 0,
            error: None,
            created_at: "1000Z".to_string(),
            started_at: None,
            completed_at: None,
        }
    }

    #[test]
    fn in_memory_opens_without_error() {
        let store = JobStore::in_memory().unwrap();
        assert_eq!(store.count_by_status("pending"), 0);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let store = JobStore::in_memory().unwrap();

        let mut job = make_job("job_1", JobStatus::Pending);
        job.priority = Priority::High;
        job.error = Some("oops".into());
        job.started_at = Some("2000Z".into());
        job.completed_at = Some("3000Z".into());
        job.delay_secs = 10;
        job.retry_count = 2;
        job.max_retries = 5;
        job.queue = "emails".to_string();

        store.save(&job).unwrap();

        let loaded = store.load("job_1").unwrap().unwrap();
        assert_eq!(loaded.id, "job_1");
        assert_eq!(loaded.name, "test_job");
        assert_eq!(loaded.payload, serde_json::json!({"key": "value"}));
        assert_eq!(loaded.priority, Priority::High);
        assert_eq!(loaded.status, JobStatus::Pending);
        assert_eq!(loaded.max_retries, 5);
        assert_eq!(loaded.retry_count, 2);
        assert_eq!(loaded.queue, "emails");
        assert_eq!(loaded.delay_secs, 10);
        assert_eq!(loaded.error, Some("oops".into()));
        assert_eq!(loaded.created_at, "1000Z");
        assert_eq!(loaded.started_at, Some("2000Z".into()));
        assert_eq!(loaded.completed_at, Some("3000Z".into()));
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let store = JobStore::in_memory().unwrap();
        assert!(store.load("nonexistent").unwrap().is_none());
    }

    #[test]
    fn save_updates_existing_job() {
        let store = JobStore::in_memory().unwrap();

        let mut job = make_job("job_1", JobStatus::Pending);
        store.save(&job).unwrap();

        job.status = JobStatus::Running;
        job.started_at = Some("2000Z".into());
        store.save(&job).unwrap();

        let loaded = store.load("job_1").unwrap().unwrap();
        assert_eq!(loaded.status, JobStatus::Running);
        assert_eq!(loaded.started_at, Some("2000Z".into()));
    }

    #[test]
    fn load_pending_returns_actionable_jobs() {
        let store = JobStore::in_memory().unwrap();

        store.save(&make_job("j1", JobStatus::Pending)).unwrap();
        store.save(&make_job("j2", JobStatus::Running)).unwrap();
        store.save(&make_job("j3", JobStatus::Retrying)).unwrap();
        store.save(&make_job("j4", JobStatus::Completed)).unwrap();
        store.save(&make_job("j5", JobStatus::Dead)).unwrap();

        let pending = store.load_pending().unwrap();
        assert_eq!(pending.len(), 3);
        let ids: Vec<&str> = pending.iter().map(|j| j.id.as_str()).collect();
        assert!(ids.contains(&"j1"));
        assert!(ids.contains(&"j2"));
        assert!(ids.contains(&"j3"));
    }

    #[test]
    fn load_pending_orders_by_priority_then_created_at() {
        let store = JobStore::in_memory().unwrap();

        let mut low = make_job("j_low", JobStatus::Pending);
        low.priority = Priority::Low;
        low.created_at = "1000Z".into();

        let mut high = make_job("j_high", JobStatus::Pending);
        high.priority = Priority::High;
        high.created_at = "2000Z".into();

        let mut normal = make_job("j_normal", JobStatus::Pending);
        normal.priority = Priority::Normal;
        normal.created_at = "1500Z".into();

        store.save(&low).unwrap();
        store.save(&high).unwrap();
        store.save(&normal).unwrap();

        let pending = store.load_pending().unwrap();
        assert_eq!(pending[0].id, "j_high");
        assert_eq!(pending[1].id, "j_normal");
        assert_eq!(pending[2].id, "j_low");
    }

    #[test]
    fn load_dead_returns_dead_jobs() {
        let store = JobStore::in_memory().unwrap();

        store.save(&make_job("j1", JobStatus::Dead)).unwrap();
        store.save(&make_job("j2", JobStatus::Pending)).unwrap();

        let dead = store.load_dead().unwrap();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].id, "j1");
    }

    #[test]
    fn count_by_status_counts_correctly() {
        let store = JobStore::in_memory().unwrap();

        store.save(&make_job("j1", JobStatus::Pending)).unwrap();
        store.save(&make_job("j2", JobStatus::Pending)).unwrap();
        store.save(&make_job("j3", JobStatus::Running)).unwrap();
        store.save(&make_job("j4", JobStatus::Dead)).unwrap();

        assert_eq!(store.count_by_status("pending"), 2);
        assert_eq!(store.count_by_status("running"), 1);
        assert_eq!(store.count_by_status("dead"), 1);
        assert_eq!(store.count_by_status("completed"), 0);
    }

    #[test]
    fn cleanup_completed_removes_old_jobs() {
        let store = JobStore::in_memory().unwrap();

        // A completed job with a very old completed_at timestamp.
        let mut old = make_job("j_old", JobStatus::Completed);
        old.completed_at = Some("100Z".into());
        store.save(&old).unwrap();

        // A completed job with a recent timestamp (should not be cleaned).
        let mut recent = make_job("j_recent", JobStatus::Completed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        recent.completed_at = Some(format!("{now}Z"));
        store.save(&recent).unwrap();

        // A pending job (should never be cleaned regardless of age).
        store
            .save(&make_job("j_pending", JobStatus::Pending))
            .unwrap();

        // Cleanup anything completed more than 1 hour ago.
        let deleted = store.cleanup_completed(3600);
        assert_eq!(deleted, 1);

        // Old one gone, recent one remains, pending untouched.
        assert!(store.load("j_old").unwrap().is_none());
        assert!(store.load("j_recent").unwrap().is_some());
        assert!(store.load("j_pending").unwrap().is_some());
    }

    #[test]
    fn all_priorities_roundtrip() {
        let store = JobStore::in_memory().unwrap();
        for (i, prio) in [
            Priority::Low,
            Priority::Normal,
            Priority::High,
            Priority::Critical,
        ]
        .iter()
        .enumerate()
        {
            let mut job = make_job(&format!("j_{i}"), JobStatus::Pending);
            job.priority = *prio;
            store.save(&job).unwrap();
            let loaded = store.load(&format!("j_{i}")).unwrap().unwrap();
            assert_eq!(loaded.priority, *prio);
        }
    }

    #[test]
    fn all_statuses_roundtrip() {
        let store = JobStore::in_memory().unwrap();
        let statuses = [
            JobStatus::Pending,
            JobStatus::Running,
            JobStatus::Completed,
            JobStatus::Failed,
            JobStatus::Retrying,
            JobStatus::Dead,
        ];
        for (i, status) in statuses.iter().enumerate() {
            let job = make_job(&format!("j_{i}"), status.clone());
            store.save(&job).unwrap();
            let loaded = store.load(&format!("j_{i}")).unwrap().unwrap();
            assert_eq!(loaded.status, *status);
        }
    }
}
