//! Idempotent function calls: a mutation called with a key runs once per
//! (function, key). The key, a hash of the arguments, and the mutation's
//! result are written in the mutation's own transaction, so they commit (or
//! roll back) together. A later call with the key and the same arguments
//! returns the stored result without running the mutation again; one with
//! other arguments is refused (`KEY_REUSED`). Shards use it for grants
//! (issue #33): a shard that crashes after the commit and calls again with
//! the same key gets the same result, and the item exists once.

use pylon_functions::runner::FnCallError;
use pylon_http::{DataError, StoredCall};
use sha2::{Digest, Sha256};

/// How long a stored result is kept.
pub const KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;

const CREATE_PG: &str = "CREATE TABLE IF NOT EXISTS _pylon_fn_calls (
    fn TEXT NOT NULL,
    key TEXT NOT NULL,
    args_hash TEXT NOT NULL,
    result TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (fn, key)
);
CREATE INDEX IF NOT EXISTS _pylon_fn_calls_created_idx ON _pylon_fn_calls (created_at)";

const CREATE_SQLITE: &str = "CREATE TABLE IF NOT EXISTS _pylon_fn_calls (
    fn TEXT NOT NULL,
    key TEXT NOT NULL,
    args_hash TEXT NOT NULL,
    result TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (fn, key)
);
CREATE INDEX IF NOT EXISTS _pylon_fn_calls_created_idx ON _pylon_fn_calls (created_at)";

pub fn ensure_pg(client: &mut postgres::Client) -> Result<(), DataError> {
    client
        .batch_execute(CREATE_PG)
        .map_err(pylon_storage::pg_tx_store::pg_err_to_data)
}

pub fn ensure_sqlite(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(CREATE_SQLITE).map_err(|e| e.to_string())
}

/// The hash stored with a keyed call's result: SHA-256 of the arguments'
/// JSON text, hex.
pub fn args_hash(args: &serde_json::Value) -> String {
    let digest = Sha256::digest(args.to_string().as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The answer to a keyed call whose key has a stored result: that result
/// when the arguments are the same, else `KEY_REUSED`.
pub fn answer(
    fn_name: &str,
    key: &str,
    args_hash: &str,
    stored: StoredCall,
) -> Result<serde_json::Value, FnCallError> {
    if stored.args_hash == args_hash {
        Ok(stored.result)
    } else {
        Err(FnCallError {
            code: "KEY_REUSED".into(),
            message: format!("{fn_name} ran with key {key} and other arguments; use a new key"),
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn parse(fn_name: &str, key: &str, text: &str, args_hash: String) -> Result<StoredCall, DataError> {
    let result = serde_json::from_str(text).map_err(|e| DataError {
        code: "FN_CALL_RESULT_CORRUPT".into(),
        message: format!("stored result of {fn_name} call {key}: {e}"),
    })?;
    Ok(StoredCall { result, args_hash })
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
) -> Result<Option<StoredCall>, DataError> {
    use rusqlite::OptionalExtension;
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT result, args_hash FROM _pylon_fn_calls WHERE fn = ?1 AND key = ?2",
            [fn_name, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sqlite_err)?;
    row.map(|(t, h)| parse(fn_name, key, &t, h)).transpose()
}

pub fn sqlite_record(
    conn: &rusqlite::Connection,
    fn_name: &str,
    key: &str,
    call: &StoredCall,
) -> Result<(), DataError> {
    conn.execute(
        "INSERT INTO _pylon_fn_calls (fn, key, args_hash, result, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            fn_name,
            key,
            call.args_hash,
            call.result.to_string(),
            now_ms()
        ],
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
) -> Result<Option<StoredCall>, DataError> {
    if let Some(pg) = runtime.pg_backend() {
        let row: Option<(String, String)> = pg.store.with_client(|c| {
            c.query_opt(
                "SELECT result, args_hash FROM _pylon_fn_calls WHERE fn = $1 AND key = $2",
                &[&fn_name, &key],
            )
            .map(|row| row.map(|r| (r.get(0), r.get(1))))
            .map_err(pylon_storage::pg_tx_store::pg_err_to_data)
        })?;
        return row.map(|(t, h)| parse(fn_name, key, &t, h)).transpose();
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

    /// A call with arguments `{"a": n}` that returned `{"n": n}`.
    fn call(n: i64) -> StoredCall {
        StoredCall {
            result: serde_json::json!({ "n": n }),
            args_hash: args_hash(&serde_json::json!({ "a": n })),
        }
    }

    /// A stored call answers a call with the same arguments and refuses one
    /// with others.
    #[test]
    fn a_key_answers_only_the_same_arguments() {
        let same = args_hash(&serde_json::json!({ "a": 1 }));
        assert_eq!(
            answer("grant", "k", &same, call(1)).unwrap(),
            serde_json::json!({ "n": 1 })
        );
        let other = args_hash(&serde_json::json!({ "a": 2 }));
        assert_eq!(
            answer("grant", "k", &other, call(1)).unwrap_err().code,
            "KEY_REUSED"
        );
    }

    /// A result is kept per (function, key); a second record of the pair
    /// fails; prune removes results past KEEP_MS.
    #[test]
    fn sqlite_keeps_one_result_per_function_and_key() {
        let rt = crate::Runtime::in_memory(manifest()).unwrap();
        {
            let conn = rt.lock_conn_pub().unwrap();
            assert_eq!(sqlite_result(&conn, "grant", "k").unwrap(), None);
            sqlite_record(&conn, "grant", "k", &call(1)).unwrap();
            assert!(sqlite_record(&conn, "grant", "k", &call(2)).is_err());
            // Another function's key is its own.
            sqlite_record(&conn, "other", "k", &call(3)).unwrap();
            assert_eq!(sqlite_result(&conn, "grant", "k").unwrap(), Some(call(1)));
        }
        assert_eq!(committed_result(&rt, "other", "k").unwrap(), Some(call(3)));
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
        assert_eq!(committed_result(&rt, "grant", "k").unwrap(), Some(call(1)));
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
            s.record_fn_call(&f, "k", &call(1)).unwrap();
        });
        assert_eq!(committed_result(&rt, &f, "k").unwrap(), Some(call(1)));
        assert_eq!(committed_result(&rt, "other", "k").unwrap(), None);
        in_tx(false, &|s| {
            assert_eq!(s.fn_call_result(&f, "k").unwrap(), Some(call(1)));
            // A unique violation: refused for what it says, not retried.
            let dup = s.record_fn_call(&f, "k", &call(2)).unwrap_err();
            assert_eq!(dup.code, "PG_REJECTED");
            assert!(!crate::shard_wasm::retryable_code(&dup.code));
        });
        // An exception a trigger or function raises, and a value the client
        // cannot convert, are refused, not retried.
        let raised = pg.store.with_client(|c| {
            c.batch_execute("DO $$ BEGIN RAISE EXCEPTION 'no'; END $$")
                .map_err(pylon_storage::pg_tx_store::pg_err_to_data)
        });
        assert_eq!(raised.unwrap_err().code, "PG_REJECTED");
        let wrong_type = pg.store.with_client(|c| {
            c.query("SELECT $1::int", &[&"not a number"])
                .map(|_| ())
                .map_err(pylon_storage::pg_tx_store::pg_err_to_data)
        });
        assert_eq!(wrong_type.unwrap_err().code, "PG_REJECTED");
        // Rolled back: no result.
        in_tx(false, &|s| s.record_fn_call(&f, "k2", &call(3)).unwrap());
        assert_eq!(committed_result(&rt, &f, "k2").unwrap(), None);
        assert_eq!(committed_result(&rt, &f, "k").unwrap(), Some(call(1)));
    }
}
