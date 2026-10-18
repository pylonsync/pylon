//! `afterEdit` hooks: an internal function that runs after edits to a row
//! land (a search index, a preview, a review queue).
//!
//! Every write to an entity with a hook notes the row in the outbox table
//! (`pylon_kernel::AFTER_EDIT_TABLE`) inside the write's own transaction:
//! one row per (entity, row, window end), so the writes of one window share
//! it. A committed write always has its row; a rolled-back one never does.
//! [`dispatch_due`] turns due rows into jobs that call the function with
//! `{ id }`, and deletes them:
//!
//! - on Postgres the job insert and the delete are one transaction, with
//!   `FOR UPDATE SKIP LOCKED`, so machines split the work;
//! - on SQLite the jobs live in their own database file, so the job is
//!   enqueued first and the row deleted after. A crash in between enqueues
//!   the same job id again, which the job store refuses as a duplicate.
//!
//! A streamed AI draft that commits many times a second runs the hook once
//! per window.

use std::sync::Arc;

use rusqlite::{params, Connection};

use crate::jobs::{Job, JobQueue, Priority};

/// Due rows taken per dispatch pass.
const DISPATCH_BATCH: i64 = 500;

// `gen` is random per inserted row and kept by the writes that merge into
// it: the job id carries it, so a row put back after its window was
// dispatched (a write at the window's very end) gets a job of its own.
const CREATE_SQLITE: &str = "CREATE TABLE IF NOT EXISTS _pylon_after_edit (
    entity TEXT NOT NULL,
    row_id TEXT NOT NULL,
    due_at INTEGER NOT NULL,
    gen INTEGER NOT NULL,
    PRIMARY KEY (entity, row_id, due_at)
);
CREATE INDEX IF NOT EXISTS _pylon_after_edit_due ON _pylon_after_edit (due_at)";

const CREATE_PG: &str = "CREATE TABLE IF NOT EXISTS _pylon_after_edit (
    entity text NOT NULL,
    row_id text NOT NULL,
    due_at bigint NOT NULL,
    gen bigint NOT NULL,
    PRIMARY KEY (entity, row_id, due_at)
);
CREATE INDEX IF NOT EXISTS _pylon_after_edit_due ON _pylon_after_edit (due_at);
ALTER TABLE _pylon_after_edit ADD COLUMN IF NOT EXISTS gen bigint NOT NULL DEFAULT 0";

/// Create the outbox in a SQLite app database. Called when the runtime
/// opens, before any write.
pub fn ensure_table_sqlite(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(CREATE_SQLITE)
        .map_err(|e| format!("create {}: {e}", pylon_kernel::AFTER_EDIT_TABLE))?;
    // A table made before `gen` (an unreleased build) gets the column.
    let has_gen: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('_pylon_after_edit') WHERE name = 'gen')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("read {} columns: {e}", pylon_kernel::AFTER_EDIT_TABLE))?;
    if has_gen {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE _pylon_after_edit ADD COLUMN gen INTEGER NOT NULL DEFAULT 0",
        [],
    )
    .map(|_| ())
    .map_err(|e| format!("create {}: {e}", pylon_kernel::AFTER_EDIT_TABLE))
}

/// Create the outbox in a Postgres app database.
pub fn ensure_table_pg(client: &mut postgres::Client) -> Result<(), pylon_http::DataError> {
    client
        .batch_execute(CREATE_PG)
        .map_err(pylon_storage::pg_tx_store::pg_err_to_data)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Note the row's run in the write's own transaction (SQLite). The
/// Postgres twin is `pylon_storage::pg_tx_store::record_after_edit`.
pub(crate) fn record_sqlite(
    conn: &Connection,
    manifest: &pylon_kernel::AppManifest,
    entity: &str,
    row_id: &str,
) -> Result<(), String> {
    let Some(hook) = hook_for(manifest, entity) else {
        return Ok(());
    };
    conn.prepare_cached(
        "INSERT OR IGNORE INTO _pylon_after_edit (entity, row_id, due_at, gen) VALUES (?1, ?2, ?3, ?4)",
    )
    .and_then(|mut stmt| stmt.execute(params![entity, row_id, hook.due_at(now_secs()) as i64, new_gen()]))
    .map(|_| ())
    .map_err(|e| format!("note the afterEdit run: {e}"))
}

/// A row generation: random, so rows put back after a dispatch never repeat
/// one.
pub fn new_gen() -> i64 {
    rand::random::<i64>()
}

fn hook_for<'a>(
    manifest: &'a pylon_kernel::AppManifest,
    entity: &str,
) -> Option<&'a pylon_kernel::ManifestAfterEdit> {
    manifest
        .entities
        .iter()
        .find(|e| e.name == entity)
        .and_then(|e| e.after_edit.as_ref())
}

/// The job for a due outbox row: its id names the row and the window.
pub(crate) fn due_job(
    jobs: &JobQueue,
    manifest: &pylon_kernel::AppManifest,
    entity: &str,
    row_id: &str,
    due_at: i64,
    gen: i64,
) -> Option<Job> {
    let hook = hook_for(manifest, entity)?;
    let mut job = jobs.new_job(
        &hook.function,
        serde_json::json!({ "id": row_id }),
        Priority::Normal,
        0,
        3,
        "functions",
        None,
    );
    job.id = format!("afterEdit:{entity}:{row_id}:{due_at}:{gen:x}");
    Some(job)
}

/// Turn due outbox rows into jobs and delete them. Returns how many it
/// took. A row of an entity that no longer has a hook is deleted.
pub fn dispatch_due(runtime: &crate::Runtime, jobs: &JobQueue) -> Result<usize, String> {
    let now = now_secs() as i64;
    let manifest = runtime.manifest();
    // The Postgres queue: the job and the delete commit together. Any other
    // queue (SQLite's, or the in-memory one PYLON_JOBS_IN_MEMORY selects)
    // takes the enqueue-then-delete path below.
    if let (Some(pg), true) = (runtime.pg_data_store_pub(), jobs.is_distributed()) {
        return pg
            .with_client(|client| -> Result<usize, pylon_http::DataError> {
                let pg_err = |e: postgres::Error| pylon_http::DataError {
                    code: "AFTER_EDIT_DISPATCH".into(),
                    message: e.to_string(),
                };
                let mut tx = client.transaction().map_err(pg_err)?;
                let rows = tx
                    .query(
                        "SELECT entity, row_id, due_at, gen FROM _pylon_after_edit \
                         WHERE due_at <= $1 ORDER BY due_at LIMIT $2 FOR UPDATE SKIP LOCKED",
                        &[&now, &DISPATCH_BATCH],
                    )
                    .map_err(pg_err)?;
                for row in &rows {
                    let (entity, row_id, due_at, gen): (String, String, i64, i64) =
                        (row.get(0), row.get(1), row.get(2), row.get(3));
                    if let Some(job) = due_job(jobs, manifest, &entity, &row_id, due_at, gen) {
                        crate::pg_job_store::insert_job(&mut tx, &job).map_err(pg_err)?;
                    }
                    tx.execute(
                        "DELETE FROM _pylon_after_edit \
                         WHERE entity = $1 AND row_id = $2 AND due_at = $3",
                        &[&entity, &row_id, &due_at],
                    )
                    .map_err(pg_err)?;
                }
                tx.commit().map_err(pg_err)?;
                Ok(rows.len())
            })
            .map_err(|e| e.message);
    }
    let due = due_rows(runtime, now)?;
    for (entity, row_id, due_at, gen) in &due {
        // Enqueued (or already there) before the row goes: a crash between
        // the two enqueues the same id again, which the queue refuses.
        if let Some(job) = due_job(jobs, manifest, entity, row_id, *due_at, *gen) {
            jobs.try_enqueue_once(job)?;
        }
        delete_row(runtime, entity, row_id, *due_at)?;
    }
    Ok(due.len())
}

type OutboxRow = (String, String, i64, i64);

/// Due outbox rows, oldest first.
fn due_rows(runtime: &crate::Runtime, now: i64) -> Result<Vec<OutboxRow>, String> {
    const SQL_PG: &str = "SELECT entity, row_id, due_at, gen FROM _pylon_after_edit \
                          WHERE due_at <= $1 ORDER BY due_at LIMIT $2";
    if let Some(pg) = runtime.pg_data_store_pub() {
        return pg
            .with_client(|c| -> Result<Vec<OutboxRow>, pylon_http::DataError> {
                Ok(c.query(SQL_PG, &[&now, &DISPATCH_BATCH])
                    .map_err(pylon_storage::pg_tx_store::pg_err_to_data)?
                    .iter()
                    .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
                    .collect())
            })
            .map_err(|e| e.message);
    }
    let conn = runtime.lock_conn_pub().map_err(|e| e.message)?;
    let mut stmt = conn
        .prepare(
            "SELECT entity, row_id, due_at, gen FROM _pylon_after_edit \
             WHERE due_at <= ?1 ORDER BY due_at LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![now, DISPATCH_BATCH], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string());
    rows
}

fn delete_row(
    runtime: &crate::Runtime,
    entity: &str,
    row_id: &str,
    due_at: i64,
) -> Result<(), String> {
    if let Some(pg) = runtime.pg_data_store_pub() {
        return pg
            .with_client(|c| -> Result<(), pylon_http::DataError> {
                c.execute(
                    "DELETE FROM _pylon_after_edit WHERE entity = $1 AND row_id = $2 AND due_at = $3",
                    &[&entity, &row_id, &due_at],
                )
                .map(|_| ())
                .map_err(pylon_storage::pg_tx_store::pg_err_to_data)
            })
            .map_err(|e| e.message);
    }
    let conn = runtime.lock_conn_pub().map_err(|e| e.message)?;
    conn.execute(
        "DELETE FROM _pylon_after_edit WHERE entity = ?1 AND row_id = ?2 AND due_at = ?3",
        params![entity, row_id, due_at],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Start the dispatcher: one pass a second. Weak references, so the
/// thread ends with the server. Does nothing when no entity has a hook.
pub fn start_dispatcher(runtime: &Arc<crate::Runtime>, jobs: &Arc<JobQueue>) {
    if !runtime
        .manifest()
        .entities
        .iter()
        .any(|e| e.after_edit.is_some())
    {
        return;
    }
    let runtime = Arc::downgrade(runtime);
    let jobs = Arc::downgrade(jobs);
    let _ = std::thread::Builder::new()
        .name("pylon-after-edit".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let (Some(runtime), Some(jobs)) = (runtime.upgrade(), jobs.upgrade()) else {
                break;
            };
            match dispatch_due(&runtime, &jobs) {
                Ok(n) if n > 0 => tracing::debug!("[afterEdit] dispatched {n} run(s)"),
                Ok(_) => {}
                Err(e) => tracing::warn!("[afterEdit] dispatch: {e}"),
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_in_one_window_share_its_end() {
        let hook = pylon_kernel::ManifestAfterEdit {
            function: "f".into(),
            delay_secs: 5,
        };
        assert_eq!(hook.due_at(100), 105);
        assert_eq!(hook.due_at(104), 105);
        assert_eq!(hook.due_at(105), 110);
        let zero = pylon_kernel::ManifestAfterEdit {
            function: "f".into(),
            delay_secs: 0,
        };
        assert_eq!(zero.due_at(7), 8, "a zero window is one second");
    }

    fn manifest_with_hook() -> pylon_kernel::AppManifest {
        pylon_kernel::AppManifest {
            entities: vec![
                pylon_kernel::ManifestEntity {
                    name: "Doc".into(),
                    after_edit: Some(pylon_kernel::ManifestAfterEdit {
                        function: "indexDoc".into(),
                        delay_secs: 30,
                    }),
                    ..Default::default()
                },
                pylon_kernel::ManifestEntity {
                    name: "Other".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn many_writes_note_one_run_per_row_and_window() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_table_sqlite(&conn).unwrap();
        let m = manifest_with_hook();
        for _ in 0..20 {
            record_sqlite(&conn, &m, "Doc", "d1").unwrap();
        }
        record_sqlite(&conn, &m, "Doc", "d2").unwrap();
        record_sqlite(&conn, &m, "Other", "o1").unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM _pylon_after_edit", [], |r| r.get(0))
            .unwrap();
        assert!(
            rows == 2 || rows == 3,
            "one per row and window (a window may turn): {rows}"
        );
        let other: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _pylon_after_edit WHERE entity = 'Other'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(other, 0, "Other has no hook");
    }

    #[test]
    fn a_due_row_becomes_one_job_named_for_its_window() {
        let jobs = JobQueue::new(10);
        let m = manifest_with_hook();
        let job = due_job(&jobs, &m, "Doc", "d1", 120, 255).unwrap();
        assert_eq!(job.id, "afterEdit:Doc:d1:120:ff");
        assert_eq!(
            (job.name.as_str(), job.queue.as_str()),
            ("indexDoc", "functions")
        );
        assert_eq!(job.payload, serde_json::json!({ "id": "d1" }));
        assert!(job.auth.is_none(), "runs without a user, like a cron");
        assert!(jobs.try_enqueue_once(job.clone()).unwrap());
        assert!(
            !jobs.try_enqueue_once(job).unwrap(),
            "the same window is refused"
        );
        assert!(due_job(&jobs, &m, "Other", "o1", 120, 1).is_none());
        // The same window put back after its dispatch: a new generation,
        // a job of its own.
        let again = due_job(&jobs, &m, "Doc", "d1", 120, 256).unwrap();
        assert!(jobs.try_enqueue_once(again).unwrap());
    }
}
