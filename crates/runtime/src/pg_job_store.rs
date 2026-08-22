//! Shared Postgres storage and distributed claims for background jobs.
//!
//! The table is internal to the runtime. It is not an app entity and it is
//! never replicated to clients. Each claim is a short transaction. User code
//! runs after that transaction commits and renews a bounded lease while it is
//! active. This gives at-least-once execution and lets another replica recover
//! work after a process or machine failure.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use pylon_storage::pg_datastore::PgPool;

use crate::jobs::{Job, JobAuth, JobStatus, Priority, QueueStats};

const JOB_SCHEMA_LOCK_ID: i64 = 0x5059_4A4F;

pub struct PgJobStore {
    pool: Arc<PgPool>,
    owner: String,
    claim_seq: AtomicU64,
}

impl PgJobStore {
    pub fn open(pool: Arc<PgPool>, owner: String) -> Result<Self, String> {
        let store = Self {
            pool,
            owner,
            claim_seq: AtomicU64::new(1),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), String> {
        self.pool.with_client(|client| {
            let mut tx = client.transaction()?;
            tx.execute("SELECT pg_advisory_xact_lock($1)", &[&JOB_SCHEMA_LOCK_ID])?;
            tx.batch_execute(
                "CREATE TABLE IF NOT EXISTS _pylon_jobs (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    payload JSONB NOT NULL,
                    priority SMALLINT NOT NULL DEFAULT 1,
                    status TEXT NOT NULL DEFAULT 'pending',
                    max_retries INTEGER NOT NULL DEFAULT 3,
                    retry_count INTEGER NOT NULL DEFAULT 0,
                    queue TEXT NOT NULL DEFAULT 'default',
                    delay_secs BIGINT NOT NULL DEFAULT 0,
                    ready_at BIGINT NOT NULL,
                    error TEXT,
                    created_at BIGINT NOT NULL,
                    started_at BIGINT,
                    completed_at BIGINT,
                    auth JSONB,
                    lease_owner TEXT,
                    lease_token TEXT,
                    lease_expires_at BIGINT
                );
                CREATE INDEX IF NOT EXISTS _pylon_jobs_ready_idx
                    ON _pylon_jobs (queue, priority DESC, ready_at, created_at)
                    WHERE status IN ('pending', 'retrying');
                CREATE INDEX IF NOT EXISTS _pylon_jobs_expired_lease_idx
                    ON _pylon_jobs (lease_expires_at)
                    WHERE status = 'running';
                CREATE INDEX IF NOT EXISTS _pylon_jobs_terminal_idx
                    ON _pylon_jobs (completed_at)
                    WHERE status IN ('completed', 'dead');",
            )?;
            tx.commit()
        })
    }

    pub fn enqueue(&self, job: &Job) -> Result<(), String> {
        let created_at = parse_stamp(&job.created_at);
        let ready_at = if job.ready_at == 0 {
            created_at.saturating_add(job.delay_secs)
        } else {
            job.ready_at
        } as i64;
        let created_at = created_at as i64;
        let priority = priority_to_i16(job.priority);
        let status = status_to_str(&job.status);
        let delay_secs = job.delay_secs.min(i64::MAX as u64) as i64;
        let max_retries = job.max_retries.min(i32::MAX as u32) as i32;
        let retry_count = job.retry_count.min(i32::MAX as u32) as i32;
        let auth = job
            .auth
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| format!("job auth serialize failed: {e}"))?;
        self.pool.with_client(|client| {
            client.execute(
                "INSERT INTO _pylon_jobs
                 (id, name, payload, priority, status, max_retries, retry_count,
                  queue, delay_secs, ready_at, error, created_at, started_at,
                  completed_at, auth)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
                 ON CONFLICT (id) DO NOTHING",
                &[
                    &job.id,
                    &job.name,
                    &job.payload,
                    &priority,
                    &status,
                    &max_retries,
                    &retry_count,
                    &job.queue,
                    &delay_secs,
                    &ready_at,
                    &job.error,
                    &created_at,
                    &job.started_at.as_deref().map(parse_stamp_i64),
                    &job.completed_at.as_deref().map(parse_stamp_i64),
                    &auth,
                ],
            )?;
            Ok(())
        })
    }

    pub fn claim(
        &self,
        queue: Option<&str>,
        handlers: &[String],
        lease_secs: u64,
    ) -> Result<Option<(Job, String)>, String> {
        if handlers.is_empty() {
            return Ok(None);
        }
        let token = format!(
            "{}-{}",
            self.owner,
            self.claim_seq.fetch_add(1, Ordering::Relaxed)
        );
        let owner = self.owner.clone();
        let lease_secs = lease_secs.min(i64::MAX as u64) as i64;
        self.pool.with_client_once(|client| {
            let mut tx = client.transaction()?;
            let row = tx.query_opt(
                "WITH candidate AS (
                    SELECT id FROM _pylon_jobs
                    WHERE (
                        (status IN ('pending','retrying') AND ready_at <= EXTRACT(EPOCH FROM clock_timestamp())::BIGINT)
                        OR
                        (status = 'running' AND lease_expires_at < EXTRACT(EPOCH FROM clock_timestamp())::BIGINT)
                    )
                    AND ($1::TEXT IS NULL OR queue = $1)
                    AND name = ANY($5)
                    ORDER BY priority DESC, ready_at ASC, created_at ASC
                    FOR UPDATE SKIP LOCKED
                    LIMIT 1
                 )
                 UPDATE _pylon_jobs AS jobs
                 SET status = 'running',
                     started_at = EXTRACT(EPOCH FROM clock_timestamp())::BIGINT,
                     lease_owner = $2,
                     lease_token = $3,
                     lease_expires_at = EXTRACT(EPOCH FROM clock_timestamp())::BIGINT + $4
                 FROM candidate
                 WHERE jobs.id = candidate.id
                 RETURNING jobs.id, jobs.name, jobs.payload, jobs.priority,
                           jobs.status, jobs.max_retries, jobs.retry_count,
                           jobs.queue, jobs.delay_secs, jobs.ready_at, jobs.error,
                           jobs.created_at, jobs.started_at, jobs.completed_at,
                           jobs.auth",
                &[&queue, &owner, &token, &lease_secs, &handlers],
            )?;
            tx.commit()?;
            Ok(row.map(|row| (row_to_job(&row), token.clone())))
        })
    }

    pub fn heartbeat(&self, id: &str, token: &str, lease_secs: u64) -> Result<bool, String> {
        let lease_secs = lease_secs.min(i64::MAX as u64) as i64;
        self.pool.with_client(|client| {
            client
                .execute(
                    "UPDATE _pylon_jobs
                     SET lease_expires_at = EXTRACT(EPOCH FROM clock_timestamp())::BIGINT + $3
                     WHERE id = $1 AND status = 'running' AND lease_token = $2",
                    &[&id, &token, &lease_secs],
                )
                .map(|n| n == 1)
        })
    }

    pub fn complete(&self, id: &str, token: &str) -> Result<bool, String> {
        self.pool.with_client(|client| {
            client
                .execute(
                    "UPDATE _pylon_jobs
                     SET status = 'completed',
                         completed_at = EXTRACT(EPOCH FROM clock_timestamp())::BIGINT,
                         lease_owner = NULL, lease_token = NULL, lease_expires_at = NULL
                     WHERE id = $1 AND status = 'running' AND lease_token = $2",
                    &[&id, &token],
                )
                .map(|n| n == 1)
        })
    }

    pub fn fail(
        &self,
        job: &Job,
        token: &str,
        error: &str,
        ready_at: u64,
    ) -> Result<Option<JobStatus>, String> {
        let retrying = job.retry_count < job.max_retries;
        let status = if retrying { "retrying" } else { "dead" };
        let retry_count = if retrying {
            job.retry_count.saturating_add(1)
        } else {
            job.retry_count
        }
        .min(i32::MAX as u32) as i32;
        let completed_at: Option<i64> = (!retrying).then(now_secs_i64);
        let ready_at = ready_at.min(i64::MAX as u64) as i64;
        self.pool.with_client(|client| {
            let changed = client.execute(
                "UPDATE _pylon_jobs
                 SET status = $3, retry_count = $4, error = $5, ready_at = $6,
                     started_at = NULL, completed_at = $7,
                     lease_owner = NULL, lease_token = NULL, lease_expires_at = NULL
                 WHERE id = $1 AND status = 'running' AND lease_token = $2",
                &[
                    &job.id,
                    &token,
                    &status,
                    &retry_count,
                    &error,
                    &ready_at,
                    &completed_at,
                ],
            )?;
            Ok((changed == 1).then_some(if retrying {
                JobStatus::Retrying
            } else {
                JobStatus::Dead
            }))
        })
    }

    pub fn load(&self, id: &str) -> Result<Option<Job>, String> {
        self.pool.with_client(|client| {
            client
                .query_opt(
                    "SELECT id,name,payload,priority,status,max_retries,retry_count,
                            queue,delay_secs,ready_at,error,created_at,started_at,
                            completed_at,auth
                     FROM _pylon_jobs WHERE id=$1",
                    &[&id],
                )
                .map(|row| row.map(|r| row_to_job(&r)))
        })
    }

    pub fn list(
        &self,
        status: Option<&str>,
        queue: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Job>, String> {
        let limit = limit.min(i64::MAX as usize) as i64;
        self.pool.with_client(|client| {
            client
                .query(
                    "SELECT id,name,payload,priority,status,max_retries,retry_count,
                            queue,delay_secs,ready_at,error,created_at,started_at,
                            completed_at,auth
                     FROM _pylon_jobs
                     WHERE ($1::TEXT IS NULL OR status=$1)
                       AND ($2::TEXT IS NULL OR queue=$2)
                     ORDER BY created_at DESC LIMIT $3",
                    &[&status, &queue, &limit],
                )
                .map(|rows| rows.iter().map(row_to_job).collect())
        })
    }

    pub fn stats(&self, handlers: Vec<String>) -> Result<QueueStats, String> {
        self.pool.with_client(|client| {
            let row = client.query_one(
                "SELECT
                    COUNT(*) FILTER (WHERE status IN ('pending','retrying')),
                    COUNT(*) FILTER (WHERE status='running'),
                    COUNT(*) FILTER (WHERE status='completed'),
                    COUNT(*) FILTER (WHERE status='dead')
                 FROM _pylon_jobs",
                &[],
            )?;
            Ok(QueueStats {
                pending: count_to_usize(row.get::<_, i64>(0)),
                running: count_to_usize(row.get::<_, i64>(1)),
                completed: row.get::<_, i64>(2).max(0) as u64,
                failed: row.get::<_, i64>(3).max(0) as u64,
                dead: count_to_usize(row.get::<_, i64>(3)),
                handlers: handlers.clone(),
            })
        })
    }

    pub fn retry_dead(&self, id: &str) -> Result<bool, String> {
        self.pool.with_client(|client| {
            client
                .execute(
                    "UPDATE _pylon_jobs
                     SET status='pending', retry_count=0, error=NULL, ready_at=0,
                         started_at=NULL, completed_at=NULL
                     WHERE id=$1 AND status='dead'",
                    &[&id],
                )
                .map(|n| n == 1)
        })
    }

    pub fn cleanup_completed(&self, max_age_secs: u64) -> Result<usize, String> {
        let cutoff = now_secs_i64().saturating_sub(max_age_secs.min(i64::MAX as u64) as i64);
        self.pool.with_client(|client| {
            client
                .execute(
                    "DELETE FROM _pylon_jobs
                     WHERE status IN ('completed','dead') AND completed_at < $1",
                    &[&cutoff],
                )
                .map(|n| n as usize)
        })
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }
}

fn row_to_job(row: &postgres::Row) -> Job {
    let created_at = row.get::<_, i64>(11).max(0) as u64;
    Job {
        id: row.get(0),
        name: row.get(1),
        payload: row.get(2),
        priority: priority_from_i16(row.get(3)),
        status: status_from_str(row.get::<_, String>(4).as_str()),
        max_retries: row.get::<_, i32>(5).max(0) as u32,
        retry_count: row.get::<_, i32>(6).max(0) as u32,
        queue: row.get(7),
        delay_secs: row.get::<_, i64>(8).max(0) as u64,
        ready_at: row.get::<_, i64>(9).max(0) as u64,
        error: row.get(10),
        created_at: format!("{created_at}Z"),
        started_at: row
            .get::<_, Option<i64>>(12)
            .map(|v| format!("{}Z", v.max(0))),
        completed_at: row
            .get::<_, Option<i64>>(13)
            .map(|v| format!("{}Z", v.max(0))),
        auth: row
            .get::<_, Option<serde_json::Value>>(14)
            .and_then(|v| serde_json::from_value::<JobAuth>(v).ok()),
    }
}

fn priority_to_i16(priority: Priority) -> i16 {
    match priority {
        Priority::Low => 0,
        Priority::Normal => 1,
        Priority::High => 2,
        Priority::Critical => 3,
    }
}

fn priority_from_i16(priority: i16) -> Priority {
    match priority {
        0 => Priority::Low,
        2 => Priority::High,
        3 => Priority::Critical,
        _ => Priority::Normal,
    }
}

fn status_to_str(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Pending => "pending",
        JobStatus::Running => "running",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::Retrying => "retrying",
        JobStatus::Dead => "dead",
    }
}

fn status_from_str(status: &str) -> JobStatus {
    match status {
        "running" => JobStatus::Running,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        "retrying" => JobStatus::Retrying,
        "dead" => JobStatus::Dead,
        _ => JobStatus::Pending,
    }
}

fn parse_stamp(value: &str) -> u64 {
    value.trim_end_matches('Z').parse().unwrap_or(0)
}

fn parse_stamp_i64(value: &str) -> i64 {
    parse_stamp(value).min(i64::MAX as u64) as i64
}

fn now_secs_i64() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

fn count_to_usize(value: i64) -> usize {
    value.max(0).min(usize::MAX as i64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_http::DataError;
    use pylon_kernel::AppManifest;
    use pylon_storage::pg_datastore::PostgresDataStore;
    use std::time::Duration;

    fn test_pool() -> Option<Arc<PgPool>> {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return None;
        };
        Some(PgPool::connect(&url, 2, Duration::from_secs(5)).expect("test Postgres pool"))
    }

    fn test_job(id: String, name: String) -> Job {
        Job {
            id,
            name,
            payload: serde_json::json!({"value": 1}),
            priority: Priority::Normal,
            status: JobStatus::Pending,
            max_retries: 3,
            retry_count: 0,
            created_at: format!("{}Z", now_secs_i64()),
            started_at: None,
            completed_at: None,
            error: None,
            delay_secs: 0,
            ready_at: 0,
            queue: "distributed-test".into(),
            auth: None,
        }
    }

    #[test]
    fn claim_is_exclusive_and_an_expired_lease_is_recoverable() {
        let Some(pool) = test_pool() else {
            return;
        };
        let suffix = pylon_cluster::new_instance_id();
        let id = format!("job_test_{suffix}");
        let name = format!("handler_test_{suffix}");
        let handlers = vec![name.clone()];
        let first =
            PgJobStore::open(Arc::clone(&pool), format!("first_{suffix}")).expect("first store");
        let second =
            PgJobStore::open(Arc::clone(&pool), format!("second_{suffix}")).expect("second store");

        first.enqueue(&test_job(id.clone(), name)).expect("enqueue");
        let (claimed, first_token) = first
            .claim(Some("distributed-test"), &handlers, 30)
            .expect("first claim")
            .expect("job available");
        assert_eq!(claimed.id, id);
        assert!(second
            .claim(Some("distributed-test"), &handlers, 30)
            .expect("contending claim")
            .is_none());

        pool.with_client(|client| {
            client.execute(
                "UPDATE _pylon_jobs SET lease_expires_at=0 WHERE id=$1",
                &[&id],
            )?;
            Ok(())
        })
        .expect("expire lease");

        let (_, second_token) = second
            .claim(Some("distributed-test"), &handlers, 30)
            .expect("recovery claim")
            .expect("expired job available");
        assert_ne!(first_token, second_token);
        assert!(!first.complete(&id, &first_token).expect("stale completion"));
        assert!(second
            .complete(&id, &second_token)
            .expect("owned completion"));

        pool.with_client(|client| {
            client.execute("DELETE FROM _pylon_jobs WHERE id=$1", &[&id])?;
            Ok(())
        })
        .expect("cleanup test job");
    }

    #[test]
    fn claim_skips_jobs_without_a_local_handler() {
        let Some(pool) = test_pool() else {
            return;
        };
        let suffix = pylon_cluster::new_instance_id();
        let id = format!("job_handler_test_{suffix}");
        let name = format!("new_release_handler_{suffix}");
        let store =
            PgJobStore::open(Arc::clone(&pool), format!("owner_{suffix}")).expect("job store");
        store
            .enqueue(&test_job(id.clone(), name.clone()))
            .expect("enqueue");

        assert!(store
            .claim(
                Some("distributed-test"),
                &["old_release_handler".into()],
                30,
            )
            .expect("old release claim")
            .is_none());
        let (_, token) = store
            .claim(Some("distributed-test"), &[name], 30)
            .expect("new release claim")
            .expect("supported job available");
        assert!(store.complete(&id, &token).expect("completion"));

        pool.with_client(|client| {
            client.execute("DELETE FROM _pylon_jobs WHERE id=$1", &[&id])?;
            Ok(())
        })
        .expect("cleanup test job");
    }

    #[test]
    fn transaction_commit_and_rollback_include_the_job_row() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let datastore =
            PostgresDataStore::connect(&url, AppManifest::default()).expect("Postgres data store");
        let suffix = pylon_cluster::new_instance_id();
        let store = PgJobStore::open(datastore.shared_pool(), format!("owner_{suffix}"))
            .expect("job store");
        let rollback_id = format!("job_tx_rollback_{suffix}");
        let committed_id = format!("job_tx_commit_{suffix}");
        let rollback_job =
            serde_json::to_value(test_job(rollback_id.clone(), format!("handler_{suffix}")))
                .expect("serialize rollback job");
        let committed_job =
            serde_json::to_value(test_job(committed_id.clone(), format!("handler_{suffix}")))
                .expect("serialize committed job");

        let rolled_back: Result<(), DataError> = datastore.with_transaction(|tx| {
            tx.enqueue_internal_job(&rollback_job)?;
            Err(DataError {
                code: "TEST_ROLLBACK".into(),
                message: "force rollback".into(),
            })
        });
        assert!(rolled_back.is_err());
        assert!(store
            .load(&rollback_id)
            .expect("load rolled-back job")
            .is_none());

        let committed: Result<(), DataError> = datastore.with_transaction(|tx| {
            tx.enqueue_internal_job(&committed_job)?;
            Ok(())
        });
        committed.expect("commit transaction");
        assert!(store
            .load(&committed_id)
            .expect("load committed job")
            .is_some());

        datastore
            .shared_pool()
            .with_client(|client| {
                client.execute(
                    "DELETE FROM _pylon_jobs WHERE id=$1 OR id=$2",
                    &[&rollback_id, &committed_id],
                )?;
                Ok(())
            })
            .expect("cleanup transaction jobs");
    }
}
