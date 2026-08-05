//! Live Postgres integration test.
//!
//! This test only runs when BOTH conditions hold:
//!
//!   1. The crate is built with `--features postgres-live`, AND
//!   2. The environment variable `TEST_POSTGRES_URL` is set to a
//!      connection string like `postgres://user:pass@localhost/dbname`.
//!
//! Otherwise the test is a no-op — so a plain `cargo test --workspace`
//! never requires Postgres to be installed.
//!
//! CI recipe (GitHub Actions):
//!
//! ```yaml
//! services:
//!   postgres:
//!     image: postgres:16
//!     env:
//!       POSTGRES_PASSWORD: test
//!     ports: ["5432:5432"]
//!     options: >-
//!       --health-cmd "pg_isready -U postgres"
//!       --health-interval 10s
//!       --health-timeout 5s
//!       --health-retries 5
//! env:
//!   TEST_POSTGRES_URL: postgres://postgres:test@localhost/postgres
//! run: cargo test -p pylon-storage --features postgres-live
//! ```

#![cfg(feature = "postgres-live")]

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pylon_http::{DataError, DataStore};
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_storage::pg_datastore::PostgresDataStore;

fn require_pg_url() -> Option<String> {
    std::env::var("TEST_POSTGRES_URL").ok()
}

/// Serializes the pool-size-sensitive tests so they don't clobber each
/// other's `PYLON_PG_POOL_SIZE` (the process env is global). Connect happens
/// inside the closure with the var set, so the returned store has the size we
/// asked for; the var is restored before the lock is released.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_pool_size<T>(size: &str, f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let prev = std::env::var("PYLON_PG_POOL_SIZE").ok();
    std::env::set_var("PYLON_PG_POOL_SIZE", size);
    let out = f();
    match prev {
        Some(v) => std::env::set_var("PYLON_PG_POOL_SIZE", v),
        None => std::env::remove_var("PYLON_PG_POOL_SIZE"),
    }
    out
}

/// Provision the manifest's tables exactly once for the whole test binary.
///
/// `PostgresDataStore::connect` does NOT create tables (production migrates
/// schema as a separate, observable `apply_plan` step — see
/// `Runtime::open_postgres`). The tests need the table to exist, so mirror
/// that step here: read the live (empty) schema, plan the diff against the
/// manifest, and apply it. `Once` makes it race-free across the parallel
/// tests; `apply_plan` is idempotent (CREATE TABLE IF NOT EXISTS) anyway.
static SCHEMA_READY: std::sync::Once = std::sync::Once::new();

fn ensure_schema(url: &str) {
    SCHEMA_READY.call_once(|| {
        use pylon_storage::postgres::live::LivePostgresAdapter;
        let mut adapter = LivePostgresAdapter::connect(url).expect("provision: connect");
        let snapshot = adapter.read_schema().expect("provision: read_schema");
        let plan = pylon_storage::postgres::plan_from_snapshot(&snapshot, &test_manifest());
        adapter.apply_plan(&plan).expect("provision: apply_plan");
    });
}

/// Delete every `PgTodo` whose title matches `marker` (test-run cleanup).
fn purge_marker(store: &PostgresDataStore, marker: &str) {
    for r in store.list("PgTodo").unwrap_or_default() {
        if r["title"] == serde_json::json!(marker) {
            if let Some(id) = r["id"].as_str() {
                let _ = store.delete("PgTodo", id);
            }
        }
    }
}

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "pg_live_test".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "PgTodo".into(),
            fields: vec![
                ManifestField {
                    name: "title".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                    sync_omit: false,
                },
                ManifestField {
                    name: "done".into(),
                    field_type: "bool".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                    sync_omit: false,
                },
            ],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
            sync: true,
            ..Default::default()
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

#[test]
fn crud_roundtrip() {
    let Some(url) = require_pg_url() else {
        eprintln!("TEST_POSTGRES_URL not set — skipping pg_live test");
        return;
    };
    ensure_schema(&url);
    let store = PostgresDataStore::connect(&url, test_manifest()).expect("connect");

    let id = store
        .insert(
            "PgTodo",
            &serde_json::json!({"title": "buy milk", "done": false}),
        )
        .expect("insert");

    let row = store.get_by_id("PgTodo", &id).expect("get").expect("row");
    assert_eq!(row["title"], "buy milk");
    assert_eq!(row["done"], false);

    let updated = store
        .update(
            "PgTodo",
            &id,
            &serde_json::json!({"title": "buy milk", "done": true}),
        )
        .expect("update");
    assert!(updated);

    let row2 = store.get_by_id("PgTodo", &id).expect("get").expect("row");
    assert_eq!(row2["done"], true);

    let rows = store.list("PgTodo").expect("list");
    assert!(rows.iter().any(|r| r["id"] == serde_json::json!(id)));

    let deleted = store.delete("PgTodo", &id).expect("delete");
    assert!(deleted);
    let gone = store.get_by_id("PgTodo", &id).expect("get after delete");
    assert!(gone.is_none());
}

#[test]
fn unknown_entity_returns_error() {
    let Some(url) = require_pg_url() else {
        eprintln!("TEST_POSTGRES_URL not set — skipping pg_live test");
        return;
    };
    let store = PostgresDataStore::connect(&url, test_manifest()).expect("connect");

    let err = store
        .insert("NotAnEntity", &serde_json::json!({}))
        .expect_err("must reject unknown entity");
    assert!(
        err.code.contains("UNKNOWN") || err.code.contains("ENTITY") || !err.message.is_empty(),
        "error should be descriptive, got code={} message={}",
        err.code,
        err.message
    );
}

/// The whole point of the pool (#337): N operations no longer serialize
/// behind one connection. Four `pg_sleep(0.25)` on a pool of 4 must run
/// concurrently (~0.25s); the old single-connection-behind-a-mutex design
/// would force them to ~1s.
#[test]
fn concurrent_ops_run_in_parallel_not_serialized() {
    let Some(url) = require_pg_url() else {
        eprintln!("TEST_POSTGRES_URL not set — skipping pg_live test");
        return;
    };
    let store = with_pool_size("4", || {
        Arc::new(PostgresDataStore::connect(&url, test_manifest()).expect("connect"))
    });

    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..4 {
        let store = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            store
                .with_client(|c| {
                    c.execute("SELECT pg_sleep(0.25)", &[])
                        .map(|_| ())
                        .map_err(|e| DataError {
                            code: "PG_TEST".into(),
                            message: e.to_string(),
                        })
                })
                .expect("pg_sleep via pooled connection");
        }));
    }
    for h in handles {
        h.join().expect("worker thread");
    }
    let elapsed = start.elapsed();

    // Serial would be ~1.0s; parallel ~0.25s + overhead. 700ms is comfortably
    // below serial yet well above a realistic parallel run, even on slow CI.
    assert!(
        elapsed < Duration::from_millis(700),
        "4× pg_sleep(0.25) on a pool of 4 took {elapsed:?} — connections are \
         serializing instead of running in parallel"
    );
}

/// Transactions pin one connection, and concurrent reads run on a DIFFERENT
/// pooled connection — so an in-flight transaction's uncommitted writes are
/// invisible to a concurrent reader (READ COMMITTED), then visible once it
/// commits. This guards the pool against leaking dirty reads and confirms the
/// reader doesn't deadlock against the transaction's pinned connection.
#[test]
fn uncommitted_tx_writes_invisible_to_concurrent_reader() {
    let Some(url) = require_pg_url() else {
        eprintln!("TEST_POSTGRES_URL not set — skipping pg_live test");
        return;
    };
    ensure_schema(&url);
    let store = with_pool_size("4", || {
        Arc::new(PostgresDataStore::connect(&url, test_manifest()).expect("connect"))
    });
    let marker = "iso-marker-337";
    purge_marker(&store, marker);

    let (sent_inserted, recv_inserted) = std::sync::mpsc::channel::<()>();
    let (sent_proceed, recv_proceed) = std::sync::mpsc::channel::<()>();

    let writer = {
        let store = Arc::clone(&store);
        thread::spawn(move || {
            store
                .with_transaction(|s| -> Result<(), DataError> {
                    s.insert(
                        "PgTodo",
                        &serde_json::json!({"title": marker, "done": false}),
                    )?;
                    // Written but NOT committed. Hold the transaction open while
                    // the reader checks a different connection.
                    sent_inserted.send(()).unwrap();
                    recv_proceed.recv().unwrap();
                    Ok(()) // commit on return
                })
                .expect("transaction");
        })
    };

    recv_inserted.recv().expect("writer signalled insert");
    let during = store
        .list("PgTodo")
        .expect("list during tx")
        .into_iter()
        .filter(|r| r["title"] == serde_json::json!(marker))
        .count();
    assert_eq!(
        during, 0,
        "an uncommitted transaction's insert leaked to a concurrent reader"
    );

    sent_proceed.send(()).expect("let writer commit");
    writer.join().expect("writer thread");

    let after = store
        .list("PgTodo")
        .expect("list after commit")
        .into_iter()
        .filter(|r| r["title"] == serde_json::json!(marker))
        .count();
    assert_eq!(after, 1, "committed insert not visible after commit");

    purge_marker(&store, marker);
}

/// A pooled connection the server drops (restart / idle timeout) must heal on
/// next use instead of permanently poisoning its slot. With a pool of 1 the
/// killed connection is the only one, so the next checkout has to reconnect it.
#[test]
fn killed_backend_heals_on_next_checkout() {
    let Some(url) = require_pg_url() else {
        eprintln!("TEST_POSTGRES_URL not set — skipping pg_live test");
        return;
    };
    ensure_schema(&url);
    let store = with_pool_size("1", || {
        PostgresDataStore::connect(&url, test_manifest()).expect("connect")
    });

    // Works before we sever anything.
    store.list("PgTodo").expect("list before kill");

    // Kill the only pooled backend from the server side. The execute itself
    // errors (the connection dies under it); the now-dead adapter returns to
    // the pool flagged closed.
    let _ = store.with_client(|c| {
        let _ = c.execute("SELECT pg_terminate_backend(pg_backend_pid())", &[]);
        Ok::<(), DataError>(())
    });

    // The next checkout must reconnect the dead slot. Retry a few times: if the
    // connection wasn't flagged closed in time on the first attempt, the failed
    // op returns it to the pool now-closed, and the next checkout heals it. If
    // heal were broken, all attempts would fail and we'd panic.
    let mut last_err = None;
    let mut healed = false;
    for attempt in 0..5 {
        match store.list("PgTodo") {
            Ok(_) => {
                healed = true;
                break;
            }
            Err(e) => {
                eprintln!("heal attempt {attempt} failed: {}", e.message);
                last_err = Some(e);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
    assert!(
        healed,
        "pool never reconnected the killed backend; last error: {:?}",
        last_err.map(|e| e.message)
    );
}

/// The auxiliary auth backends (sessions, API keys, OAuth state, …) hold ONE
/// dedicated connection for the process lifetime via `ReconnectingPgClient`.
/// Managed providers idle-kill such connections (PlanetScale did, after ~24h,
/// which silently dropped every CLI API-key INSERT on Pylon Cloud — minted
/// tokens 401'd forever). This proves the wrapper heals through a server-side
/// kill instead of failing every subsequent operation.
#[test]
fn reconnecting_client_heals_after_backend_kill() {
    let Some(url) = require_pg_url() else {
        eprintln!("TEST_POSTGRES_URL not set; skipping");
        return;
    };
    use pylon_storage::postgres::live::ReconnectingPgClient;

    let conn = ReconnectingPgClient::connect(&url).expect("connect");

    // Baseline: the connection works.
    conn.with_client(|c| c.execute("SELECT 1", &[]))
        .expect("query before kill");

    // Server-side kill — the same shape as an idle-timeout reap. The
    // statement itself errors (its own backend died under it), which is fine.
    let _ = conn.with_client(|c| c.execute("SELECT pg_terminate_backend(pg_backend_pid())", &[]));

    // The wrapper must reconnect and serve subsequent operations. Retry a few
    // times: the client may need one failed op to notice the dead socket.
    let mut healed = false;
    let mut last_err = None;
    for attempt in 0..5 {
        match conn.with_client(|c| c.execute("SELECT 1", &[])) {
            Ok(_) => {
                healed = true;
                break;
            }
            Err(e) => {
                eprintln!("heal attempt {attempt} failed: {e}");
                last_err = Some(e);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
    assert!(
        healed,
        "ReconnectingPgClient never healed after backend kill; last error: {last_err:?}"
    );
}
