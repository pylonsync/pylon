//! `db_copy`: a SQLite app's data copied into Postgres reads back the same.
//!
//! The SQLite side is built with the real runtime and stores (entities with
//! every field type, CRDT snapshots, an encrypted field, search, a session, an
//! account link, jobs, workflows, the change log). The copy runs into a fresh
//! Postgres database, then a Postgres runtime reads everything back.
//!
//! Skipped unless `PYLON_TEST_PG_URL` is set. Each test creates and drops its
//! own database, so the URL's role needs CREATEDB.
//!
//! ```sh
//! PYLON_TEST_PG_URL=postgres://localhost/postgres \
//!   cargo test -p pylon-runtime --test db_copy -- --test-threads=1
//! ```

use pylon_auth::{Account, AccountBackend, Session, SessionBackend};
use pylon_kernel::AppManifest;
use pylon_runtime::db_copy::{self, CopyError, CopyMode, CopyOutcome, SqliteSource};
use pylon_runtime::jobs::{Job, JobStatus};
use pylon_runtime::workflows::{WorkflowInstance, WorkflowStatus};
use pylon_runtime::Runtime;
use serde_json::{json, Value};
use std::time::Duration;

const ENCRYPTION_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn pg_url() -> Option<String> {
    std::env::var("PYLON_TEST_PG_URL").ok()
}

fn manifest() -> AppManifest {
    serde_json::from_value(json!({
        "manifest_version": 1,
        "name": "db_copy_test",
        "version": "1",
        "entities": [
            {
                "name": "Author",
                "fields": [{ "name": "name", "type": "string", "optional": false, "unique": true }],
                "indexes": [],
                "crdt": false
            },
            {
                "name": "Note",
                "fields": [
                    { "name": "title", "type": "string", "optional": false, "unique": false },
                    { "name": "body", "type": "richtext", "optional": false, "unique": false },
                    { "name": "views", "type": "int", "optional": false, "unique": false },
                    { "name": "score", "type": "float", "optional": false, "unique": false },
                    { "name": "pinned", "type": "bool", "optional": false, "unique": false },
                    { "name": "dueAt", "type": "datetime", "optional": true, "unique": false },
                    { "name": "createdAt", "type": "datetime", "optional": false, "unique": false },
                    { "name": "meta", "type": "json", "optional": true, "unique": false },
                    { "name": "authorId", "type": "id(Author)", "optional": true, "unique": false },
                    { "name": "embedding", "type": "vector(3)", "optional": true, "unique": false },
                    { "name": "secret", "type": "string", "optional": true, "unique": false, "encrypted": true, "serverOnly": true }
                ],
                "indexes": [],
                "search": { "text": ["title", "body"], "facets": ["pinned"], "sortable": ["views"] }
            }
        ],
        "routes": [],
        "queries": [],
        "actions": [],
        "policies": [],
        "crons": [{ "schedule": "0 * * * *", "function": "tick" }]
    }))
    .expect("manifest")
}

/// A fresh database on the test server, dropped when the guard drops.
struct TempDb {
    admin_url: String,
    name: String,
    url: String,
}

impl TempDb {
    fn create(admin_url: &str) -> Self {
        let name = format!(
            "pylon_db_copy_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let mut admin =
            pylon_storage::postgres::live::connect_pg(admin_url).expect("admin connect");
        admin
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .expect("create database");
        let (base, query) = match admin_url.split_once('?') {
            Some((b, q)) => (b, format!("?{q}")),
            None => (admin_url, String::new()),
        };
        let prefix = &base[..base.rfind('/').expect("URL has a database path")];
        Self {
            admin_url: admin_url.to_string(),
            url: format!("{prefix}/{name}{query}"),
            name,
        }
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        pylon_runtime::pg_boot_guard::release();
        if let Ok(mut admin) = pylon_storage::postgres::live::connect_pg(&self.admin_url) {
            let _ = admin.batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                self.name
            ));
        }
    }
}

fn apply_schema(url: &str, manifest: &AppManifest) {
    let mut adapter =
        pylon_storage::postgres::live::LivePostgresAdapter::connect(url).expect("adapter");
    let plan = adapter.plan_from_live(manifest).expect("plan");
    adapter.apply_plan(&plan).expect("apply");
}

fn sorted(mut rows: Vec<Value>) -> Vec<Value> {
    rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    rows
}

/// Build a SQLite app in `dir` and return its source files.
fn build_sqlite_app(dir: &std::path::Path) -> (SqliteSource, Vec<Value>, Vec<Value>, Session) {
    let app_db = dir.join("pylon.db");
    let app_db = app_db.to_str().unwrap();
    // The schema step `pylon start` runs before opening the runtime.
    let adapter = pylon_storage::sqlite::SqliteAdapter::open(app_db).expect("sqlite adapter");
    let plan = adapter.plan_from_live(&manifest()).expect("sqlite plan");
    adapter
        .apply_with_history(
            &plan,
            &pylon_storage::sqlite::PushMetadata {
                manifest_version: 1,
                app_version: "1",
                baseline: "start",
            },
        )
        .expect("sqlite apply");
    drop(adapter);
    let rt = Runtime::open(app_db, manifest()).expect("sqlite runtime");

    let author = rt
        .insert("Author", &json!({ "name": "Ada" }))
        .expect("author");
    rt.insert(
        "Note",
        &json!({
            "title": "zebra crossing",
            "body": "<p>stripes</p>",
            "views": 3,
            "score": 1.5,
            "pinned": true,
            "dueAt": "2026-11-01T09:30:00.000Z",
            "createdAt": "2026-10-05T12:00:00.000Z",
            "meta": { "tags": ["a", "b"], "n": 1 },
            "authorId": author,
            "embedding": [0.25, -1.0, 3.5],
            "secret": "hunter2"
        }),
    )
    .expect("note 1");
    rt.insert(
        "Note",
        &json!({
            "title": "second",
            "body": "",
            "views": 0,
            "score": 0.0,
            "pinned": false,
            "createdAt": "2026-10-05T12:01:00.000Z"
        }),
    )
    .expect("note 2");

    // Change log as the SQLite server persists it.
    rt.bootstrap_sqlite_change_seq().unwrap();
    rt.bootstrap_sqlite_change_log().unwrap();
    rt.reserve_sqlite_change_seq(100).expect("reserve seq");
    let events: Vec<pylon_sync::ChangeEvent> = serde_json::from_value(json!([
        { "seq": 41, "entity": "Author", "row_id": author, "kind": "insert",
          "data": { "id": author, "name": "Ada" }, "timestamp": "2026-10-05T12:00:00.000Z" },
        { "seq": 42, "entity": "Author", "row_id": author, "kind": "update",
          "data": { "id": author, "name": "Ada" }, "timestamp": "2026-10-05T12:00:01.000Z" }
    ]))
    .expect("events");
    rt.sqlite_change_log_persist_batch(&events).unwrap();

    let authors = sorted(rt.list("Author").unwrap());
    let notes = sorted(rt.list("Note").unwrap());
    drop(rt);

    // A table of an entity the manifest dropped, and saved shard state.
    let raw = rusqlite::Connection::open(app_db).unwrap();
    raw.execute_batch(
        "CREATE TABLE \"OldEntity\" (id TEXT PRIMARY KEY, x TEXT);
         INSERT INTO \"OldEntity\" VALUES ('o1', 'gone');",
    )
    .unwrap();
    drop(raw);
    pylon_runtime::shard_cluster::ShardDirectory::open_sqlite(app_db).expect("shard directory");
    let raw = rusqlite::Connection::open(app_db).unwrap();
    raw.execute(
        "INSERT INTO _pylon_shard_state (shard_id, state, saved_at) VALUES ('arena:1', x'0102', 5)",
        [],
    )
    .unwrap();
    drop(raw);

    // Auth state in the sibling sessions file.
    let source = SqliteSource {
        app_db: app_db.into(),
        sessions_db: format!("{app_db}.sessions.db").into(),
        jobs_db: format!("{app_db}.jobs.db").into(),
        workflows_db: format!("{app_db}.workflows.db").into(),
    };
    let sessions_path = source.sessions_db.to_str().unwrap();
    let session = Session::new("user_1".into());
    pylon_runtime::session_backend::SqliteSessionBackend::open(sessions_path)
        .unwrap()
        .save(&session);
    pylon_runtime::account_backend::SqliteAccountBackend::open(sessions_path)
        .unwrap()
        .upsert(&account());

    // Jobs: one running when the server stopped, one finished.
    let jobs = pylon_runtime::job_store::JobStore::open(source.jobs_db.to_str().unwrap()).unwrap();
    jobs.save(&job("job_running", JobStatus::Running)).unwrap();
    jobs.save(&job("job_done", JobStatus::Completed)).unwrap();

    // A workflow waiting for an event, with a finished step and an unread event.
    let workflows =
        pylon_runtime::workflow_store::WorkflowStore::open(source.workflows_db.to_str().unwrap())
            .unwrap();
    workflows.save(&workflow()).unwrap();

    (source, authors, notes, session)
}

fn account() -> Account {
    Account {
        id: "acct_1".into(),
        user_id: "user_1".into(),
        provider_id: "google".into(),
        account_id: "sub_1".into(),
        access_token: Some("at".into()),
        refresh_token: Some("rt".into()),
        id_token: None,
        access_token_expires_at: Some(1_800_000_000),
        refresh_token_expires_at: None,
        scope: Some("openid".into()),
        password: None,
        avatar_url: None,
        external_orgs: None,
        created_at: 1_700_000_000,
        updated_at: 1_700_000_000,
    }
}

fn job(id: &str, status: JobStatus) -> Job {
    let mut job: Job = serde_json::from_value(json!({
        "id": id, "name": "send", "payload": { "to": "x" }, "priority": "high",
        "status": "pending", "max_retries": 3, "retry_count": 1,
        "created_at": "1791201600Z", "started_at": "1791201601Z", "completed_at": null,
        "error": null, "delay_secs": 0, "ready_at": 0, "queue": "default",
        "auth": { "user_id": "user_1", "is_admin": false }
    }))
    .expect("job");
    job.status = status;
    job
}

fn workflow() -> WorkflowInstance {
    serde_json::from_value(json!({
        "id": "wf_1", "name": "leadCadence", "input": { "leadId": "l1" },
        "status": "WaitingForEvent",
        "steps": [{
            "step_id": "s0", "name": "send-first", "status": "Completed",
            "output": { "ok": true }, "error": null,
            "started_at": "1791201600Z", "completed_at": "1791201602Z",
            "duration_ms": 2000, "retry_count": 0
        }],
        "output": null, "error": null,
        "created_at": "1791201600Z", "started_at": "1791201600Z", "completed_at": null,
        "wake_at": null, "waiting_for": "reply", "current_step": 1, "max_retries": 3,
        "key": "l1", "wait_deadline": 1_900_000_000u64,
        "pending_events": [{
            "seq": 1, "event": "other", "data": { "x": 1 }, "received_at": "1791201605Z"
        }],
        "cancel_reason": null
    }))
    .expect("workflow")
}

#[test]
fn sqlite_app_copies_into_postgres() {
    let Some(admin) = pg_url() else {
        eprintln!("skipping: PYLON_TEST_PG_URL not set");
        return;
    };
    std::env::set_var("PYLON_ENCRYPTION_KEY", ENCRYPTION_KEY);
    let dir = tempfile::tempdir().unwrap();
    let (source, authors, notes, session) = build_sqlite_app(dir.path());

    let db = TempDb::create(&admin);
    apply_schema(&db.url, &manifest());
    let pg = Runtime::open(&db.url, manifest()).expect("pg runtime");

    // A check writes no rows.
    match db_copy::copy(&pg, &source, CopyMode::Check).expect("check") {
        CopyOutcome::Checked { tables, .. } => {
            let notes_count = tables.iter().find(|t| t.table == "Note").unwrap().rows;
            assert_eq!(notes_count, 2);
        }
        other => panic!("{other:?}"),
    }
    assert!(pg.list("Note").unwrap().is_empty());

    let report = match db_copy::copy(&pg, &source, CopyMode::Write).expect("copy") {
        CopyOutcome::Copied(r) => r,
        other => panic!("{other:?}"),
    };
    assert_eq!(report.jobs, 2);
    assert_eq!(report.workflows, 1);
    assert_eq!(report.change_seq, 100, "the reserved high-water mark wins");
    let skipped: Vec<_> = report
        .skipped
        .iter()
        .map(|t| (t.table.as_str(), t.rows))
        .collect();
    assert_eq!(skipped, vec![("OldEntity", 1)]);

    // Entities read back the same through the runtime: bool, datetime,
    // json, vector and the decrypted secret included.
    assert_eq!(sorted(pg.list("Author").unwrap()), authors);
    assert_eq!(sorted(pg.list("Note").unwrap()), notes);
    let first = notes
        .iter()
        .find(|n| n["title"] == "zebra crossing")
        .unwrap();
    assert_eq!(first["secret"], "hunter2");
    assert_eq!(first["pinned"], true);

    let store = pg.pg_data_store_pub().unwrap();
    store
        .with_client(|c| -> Result<(), pylon_http::DataError> {
            // Search finds the copied row.
            let hits = c
                .query(
                    "SELECT entity_id FROM \"_fts_Note\" \
                     WHERE tsv @@ plainto_tsquery('english', 'zebra')",
                    &[],
                )
                .unwrap();
            assert_eq!(hits.len(), 1);
            // The ciphertext was copied, not re-encrypted or decrypted.
            let raw: String = c
                .query_one("SELECT secret FROM \"Note\" WHERE secret IS NOT NULL", &[])
                .unwrap()
                .get(0);
            assert!(raw.starts_with("enc:"), "{raw}");
            // CRDT snapshots for the CRDT entity.
            let snaps: i64 = c
                .query_one(
                    "SELECT count(*) FROM _pylon_crdt_snapshots WHERE entity = 'Note'",
                    &[],
                )
                .unwrap()
                .get(0);
            assert_eq!(snaps, 2);
            // Change log rows and the sequence.
            let logged: i64 = c
                .query_one("SELECT count(*) FROM pylon_change_log", &[])
                .unwrap()
                .get(0);
            assert_eq!(logged, 2);
            let next: i64 = c
                .query_one("SELECT nextval('pylon_change_seq')", &[])
                .unwrap()
                .get(0);
            assert_eq!(next, 101);
            // Shard state and the SQLite cluster key, not a new one.
            let state: Vec<u8> = c
                .query_one(
                    "SELECT state FROM _pylon_shard_state WHERE shard_id = 'arena:1'",
                    &[],
                )
                .unwrap()
                .get(0);
            assert_eq!(state, vec![1u8, 2]);
            let keys: i64 = c
                .query_one("SELECT count(*) FROM _pylon_shard_cluster", &[])
                .unwrap()
                .get(0);
            assert_eq!(keys, 1);
            Ok(())
        })
        .unwrap();

    // Auth state.
    let pool =
        pylon_storage::pg_datastore::PgPool::connect(&db.url, 2, Duration::from_secs(10)).unwrap();
    let sessions = pylon_runtime::session_backend::PostgresSessionBackend::with_pool(pool.clone())
        .unwrap()
        .load_all();
    assert!(sessions
        .iter()
        .any(|s| s.token == session.token && s.user_id == "user_1"));
    let linked = pylon_runtime::account_backend::PostgresAccountBackend::with_pool(pool.clone())
        .unwrap()
        .find_by_provider("google", "sub_1")
        .expect("account copied");
    assert_eq!(linked.user_id, "user_1");
    assert_eq!(linked.refresh_token.as_deref(), Some("rt"));

    // Jobs: the running job is pending again, the finished one stays done.
    let jobs = pylon_runtime::pg_job_store::PgJobStore::open(pool.clone(), "t".into()).unwrap();
    let running = jobs.load("job_running").unwrap().unwrap();
    assert_eq!(running.status, JobStatus::Pending);
    assert_eq!(running.payload, json!({ "to": "x" }));
    assert_eq!(running.auth.unwrap().user_id.as_deref(), Some("user_1"));
    assert_eq!(
        jobs.load("job_done").unwrap().unwrap().status,
        JobStatus::Completed
    );

    // Workflows keep their step, wait and unread event.
    let wfs =
        pylon_runtime::pg_workflow_store::PgWorkflowStore::open(pool.clone(), "t".into()).unwrap();
    let wf = wfs.load("wf_1").unwrap().unwrap();
    assert!(matches!(wf.status, WorkflowStatus::WaitingForEvent));
    assert_eq!(wf.waiting_for.as_deref(), Some("reply"));
    assert_eq!(wf.steps.len(), 1);
    assert_eq!(wf.steps[0].output, Some(json!({ "ok": true })));
    assert_eq!(wf.created_at, "1791201600Z");
    let events = wfs.load_events("wf_1").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, "other");
    assert_eq!(events[0].received_at, "1791201605Z");

    // A second copy finds the marker and writes nothing.
    assert!(matches!(
        db_copy::copy(&pg, &source, CopyMode::Write).unwrap(),
        CopyOutcome::AlreadyCopied { .. }
    ));
    assert!(db_copy::is_copied(&pg).unwrap());
}

#[test]
fn bad_values_are_reported_before_anything_is_written() {
    let Some(admin) = pg_url() else {
        return;
    };
    std::env::set_var("PYLON_ENCRYPTION_KEY", ENCRYPTION_KEY);
    let dir = tempfile::tempdir().unwrap();
    let (source, _, _, _) = build_sqlite_app(dir.path());
    // An empty string where Postgres needs a timestamp: SQLite fills a new
    // required datetime column with '' on existing rows.
    let conn = rusqlite::Connection::open(&source.app_db).unwrap();
    conn.execute(
        "UPDATE \"Note\" SET \"createdAt\" = '' WHERE title = 'second'",
        [],
    )
    .unwrap();
    drop(conn);

    let db = TempDb::create(&admin);
    apply_schema(&db.url, &manifest());
    let pg = Runtime::open(&db.url, manifest()).expect("pg runtime");
    match db_copy::copy(&pg, &source, CopyMode::Write) {
        Err(CopyError::Problems { total, listed }) => {
            assert_eq!(total, 1);
            assert_eq!(listed[0].table, "Note");
            assert_eq!(listed[0].column, "createdAt");
        }
        other => panic!("{other:?}"),
    }
    assert!(pg.list("Note").unwrap().is_empty());
    assert!(!db_copy::is_copied(&pg).unwrap());

    // The boot path records the failure where the operator can read it.
    let err = db_copy::copy(&pg, &source, CopyMode::Write).unwrap_err();
    db_copy::record_failure(&pg, &err).unwrap();
    let message: String = pg
        .pg_data_store_pub()
        .unwrap()
        .with_client(|c| -> Result<String, pylon_http::DataError> {
            Ok(c.query_one(
                &format!("SELECT message FROM {}", db_copy::FAILURE_TABLE),
                &[],
            )
            .unwrap()
            .get(0))
        })
        .unwrap();
    assert!(message.contains("createdAt"), "{message}");
}

#[test]
fn a_database_with_data_is_refused() {
    let Some(admin) = pg_url() else {
        return;
    };
    std::env::set_var("PYLON_ENCRYPTION_KEY", ENCRYPTION_KEY);
    let dir = tempfile::tempdir().unwrap();
    let (source, _, _, _) = build_sqlite_app(dir.path());

    let db = TempDb::create(&admin);
    apply_schema(&db.url, &manifest());
    let pg = Runtime::open(&db.url, manifest()).expect("pg runtime");
    pg.insert("Author", &json!({ "name": "Someone else" }))
        .unwrap();
    match db_copy::copy(&pg, &source, CopyMode::Write) {
        Err(CopyError::NotEmpty { tables }) => assert_eq!(tables, vec!["Author".to_string()]),
        other => panic!("{other:?}"),
    }
    assert_eq!(pg.list("Author").unwrap().len(), 1);
}

#[test]
fn an_unknown_internal_table_is_refused() {
    let Some(admin) = pg_url() else {
        return;
    };
    std::env::set_var("PYLON_ENCRYPTION_KEY", ENCRYPTION_KEY);
    let dir = tempfile::tempdir().unwrap();
    let (source, _, _, _) = build_sqlite_app(dir.path());
    let conn = rusqlite::Connection::open(&source.app_db).unwrap();
    conn.execute_batch(
        "CREATE TABLE _pylon_future (x TEXT); INSERT INTO _pylon_future VALUES ('y');",
    )
    .unwrap();
    drop(conn);

    let db = TempDb::create(&admin);
    apply_schema(&db.url, &manifest());
    let pg = Runtime::open(&db.url, manifest()).expect("pg runtime");
    match db_copy::copy(&pg, &source, CopyMode::Check) {
        Err(CopyError::Failed(m)) => assert!(m.contains("_pylon_future"), "{m}"),
        other => panic!("{other:?}"),
    }
}
