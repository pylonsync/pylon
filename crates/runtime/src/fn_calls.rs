//! Idempotent function calls: a mutation called with a key runs once per
//! (function, key). The key and the mutation's result are written in the mutation's own
//! transaction, so they commit (or roll back) together; a later call with
//! the key returns the stored result without running the mutation again.
//! Shards use it for grants (issue #33): a shard that crashes after the
//! commit and calls again with the same key gets the same result, and the
//! item exists once.

use pylon_http::DataError;

/// How long a stored result is kept.
pub const KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;

const CREATE_PG: &str = "CREATE TABLE IF NOT EXISTS _pylon_fn_calls (
    fn TEXT NOT NULL,
    key TEXT NOT NULL,
    result TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (fn, key)
)";

const CREATE_SQLITE: &str = "CREATE TABLE IF NOT EXISTS _pylon_fn_calls (
    fn TEXT NOT NULL,
    key TEXT NOT NULL,
    result TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (fn, key)
)";

pub fn ensure_pg(client: &mut postgres::Client) -> Result<(), DataError> {
    client
        .batch_execute(CREATE_PG)
        .map_err(pylon_storage::pg_tx_store::pg_err_to_data)
}

pub fn ensure_sqlite(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(CREATE_SQLITE).map_err(|e| e.to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn parse(fn_name: &str, key: &str, text: &str) -> Result<serde_json::Value, DataError> {
    serde_json::from_str(text).map_err(|e| DataError {
        code: "FN_CALL_RESULT_CORRUPT".into(),
        message: format!("stored result of {fn_name} call {key}: {e}"),
    })
}

fn sqlite_err(e: rusqlite::Error) -> DataError {
    DataError {
        code: "SQLITE_ERROR".into(),
        message: e.to_string(),
    }
}

pub fn sqlite_result(
    conn: &rusqlite::Connection,
    fn_name: &str,
    key: &str,
) -> Result<Option<serde_json::Value>, DataError> {
    use rusqlite::OptionalExtension;
    let text: Option<String> = conn
        .query_row(
            "SELECT result FROM _pylon_fn_calls WHERE fn = ?1 AND key = ?2",
            [fn_name, key],
            |r| r.get(0),
        )
        .optional()
        .map_err(sqlite_err)?;
    text.map(|t| parse(fn_name, key, &t)).transpose()
}

pub fn sqlite_record(
    conn: &rusqlite::Connection,
    fn_name: &str,
    key: &str,
    result: &serde_json::Value,
) -> Result<(), DataError> {
    conn.execute(
        "INSERT INTO _pylon_fn_calls (fn, key, result, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![fn_name, key, result.to_string(), now_ms()],
    )
    .map_err(sqlite_err)?;
    Ok(())
}

/// The committed result of the call of `fn_name` with `key`, read outside
/// any transaction: after a call lost the race for its key, the winner's
/// result.
pub fn committed_result(
    runtime: &crate::Runtime,
    fn_name: &str,
    key: &str,
) -> Result<Option<serde_json::Value>, DataError> {
    if let Some(pg) = runtime.pg_backend() {
        let text: Option<String> = pg.store.with_client(|c| {
            c.query_opt(
                "SELECT result FROM _pylon_fn_calls WHERE fn = $1 AND key = $2",
                &[&fn_name, &key],
            )
            .map(|row| row.map(|r| r.get(0)))
            .map_err(pylon_storage::pg_tx_store::pg_err_to_data)
        })?;
        return text.map(|t| parse(fn_name, key, &t)).transpose();
    }
    let conn = runtime.lock_conn_pub().map_err(|e| DataError {
        code: "SQLITE_LOCK".into(),
        message: e.to_string(),
    })?;
    sqlite_result(&conn, fn_name, key)
}

/// Delete stored results older than [`KEEP_MS`].
pub fn prune(runtime: &crate::Runtime) -> Result<(), String> {
    let cutoff = now_ms() - KEEP_MS;
    if let Some(pg) = runtime.pg_backend() {
        return pg
            .store
            .with_client(|c| {
                c.execute(
                    "DELETE FROM _pylon_fn_calls WHERE created_at < $1",
                    &[&cutoff],
                )
                .map(|_| ())
                .map_err(pylon_storage::pg_tx_store::pg_err_to_data)
            })
            .map_err(|e: DataError| e.message);
    }
    let conn = runtime.lock_conn_pub().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM _pylon_fn_calls WHERE created_at < ?1",
        [cutoff],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_http::DataStore;

    fn manifest() -> pylon_kernel::AppManifest {
        pylon_kernel::AppManifest {
            manifest_version: pylon_kernel::MANIFEST_VERSION,
            name: "fn-calls".into(),
            version: "0".into(),
            ..Default::default()
        }
    }

    fn v(n: i64) -> serde_json::Value {
        serde_json::json!({ "n": n })
    }

    /// A result is kept per (function, key); a second record of the pair
    /// fails; prune removes results past KEEP_MS.
    #[test]
    fn sqlite_keeps_one_result_per_function_and_key() {
        let rt = crate::Runtime::in_memory(manifest()).unwrap();
        {
            let conn = rt.lock_conn_pub().unwrap();
            assert_eq!(sqlite_result(&conn, "grant", "k").unwrap(), None);
            sqlite_record(&conn, "grant", "k", &v(1)).unwrap();
            assert!(sqlite_record(&conn, "grant", "k", &v(2)).is_err());
            // Another function's key is its own.
            sqlite_record(&conn, "other", "k", &v(3)).unwrap();
            assert_eq!(sqlite_result(&conn, "grant", "k").unwrap(), Some(v(1)));
        }
        assert_eq!(committed_result(&rt, "other", "k").unwrap(), Some(v(3)));
        assert_eq!(committed_result(&rt, "grant", "nope").unwrap(), None);

        rt.lock_conn_pub()
            .unwrap()
            .execute(
                "UPDATE _pylon_fn_calls SET created_at = ?1 WHERE fn = 'other'",
                [now_ms() - KEEP_MS - 1],
            )
            .unwrap();
        prune(&rt).unwrap();
        assert_eq!(committed_result(&rt, "other", "k").unwrap(), None);
        assert_eq!(committed_result(&rt, "grant", "k").unwrap(), Some(v(1)));
    }

    /// On Postgres the result commits or rolls back with the transaction,
    /// and a second record of a committed pair fails.
    #[test]
    fn postgres_results_commit_with_the_transaction() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let rt = crate::Runtime::open_postgres(&url, manifest()).unwrap();
        let pg = rt.pg_backend().unwrap();
        let f = format!("grant-{}", pylon_cluster::new_instance_id());
        let m = manifest();
        let in_tx = |commit: bool, body: &dyn Fn(&pylon_storage::pg_tx_store::PgTxStore)| {
            pg.store
                .with_client(|c| {
                    let tx = c
                        .transaction()
                        .map_err(pylon_storage::pg_tx_store::pg_err_to_data)?;
                    let store = pylon_storage::pg_tx_store::PgTxStore::new(tx, &m);
                    body(&store);
                    if commit {
                        store
                            .commit()
                            .map_err(pylon_storage::pg_tx_store::pg_err_to_data)?;
                    }
                    Ok::<(), DataError>(())
                })
                .unwrap();
        };
        in_tx(true, &|s| {
            assert_eq!(s.fn_call_result(&f, "k").unwrap(), None);
            s.record_fn_call(&f, "k", &v(1)).unwrap();
        });
        assert_eq!(committed_result(&rt, &f, "k").unwrap(), Some(v(1)));
        assert_eq!(committed_result(&rt, "other", "k").unwrap(), None);
        in_tx(false, &|s| {
            assert_eq!(s.fn_call_result(&f, "k").unwrap(), Some(v(1)));
            assert!(s.record_fn_call(&f, "k", &v(2)).is_err());
        });
        // Rolled back: no result.
        in_tx(false, &|s| s.record_fn_call(&f, "k2", &v(3)).unwrap());
        assert_eq!(committed_result(&rt, &f, "k2").unwrap(), None);
        assert_eq!(committed_result(&rt, &f, "k").unwrap(), Some(v(1)));
    }
}
