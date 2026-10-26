//! A replication read of a scoped entity reads the rows its scope can keep,
//! not the whole table, on SQLite and on Postgres.
//!
//! The snapshot pull (`/api/sync/pull`) and the reconcile fetch
//! (`/api/entities/<E>/cursor?sync=1`) used to page through every row of
//! the entity in id order (`SELECT * ... WHERE id > $1 ORDER BY id LIMIT
//! 200`) and drop the rows outside the caller's `sync: { where }` scope in
//! memory. On an 85,000-row table each phone's reconcile read every row;
//! in production that query took 39% of the database's time, all day.
//!
//! The test seeds 3,000 rows in `Lead`, of which one user's scope keeps 40,
//! and the same rows in `Visit`, which adds `sync: { limit }` and so is read
//! newest first. It checks every read returns exactly the rows the scope
//! keeps. On Postgres a wire counter checks the reads run a few filtered
//! statements and none of the unfiltered page queries. On SQLite a small
//! scan budget makes the snapshot pause after that many rows read, so a
//! snapshot that reads out-of-scope rows takes many more pages.
//!
//! Postgres runs when `PYLON_TEST_PG_URL` is set. Shares the database with
//! the other Postgres tests: run with --test-threads=1.

#[path = "support/pg_wire.rs"]
mod pg_wire;

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::*;
use pylon_runtime::Runtime;
use serde_json::Value;

const SCOPE: &str = "data.ownerId == auth.userId \
    || (exists(Member where userId == auth.userId and teamId == data.teamId) \
        && (data.seenAt >= ago(\"30d\") || data.stage == \"won\"))";

/// The scoped entities: `Lead` is read oldest first, `Visit` (with a limit
/// above its in-scope rows) newest first.
const ENTITIES: &[&str] = &["Lead", "Visit"];

fn field(name: &str, field_type: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: field_type.into(),
        optional: true,
        ..Default::default()
    }
}

fn manifest() -> AppManifest {
    let allow_read = |entity: &str| ManifestPolicy {
        name: format!("{entity}_read"),
        entity: Some(entity.into()),
        allow_read: Some("auth.userId != null".into()),
        ..Default::default()
    };
    let scoped = |name: &str, sync_limit: Option<usize>| ManifestEntity {
        name: name.into(),
        fields: vec![
            field("ownerId", "id(User)"),
            field("teamId", "string"),
            field("stage", "string"),
            field("seenAt", "datetime"),
        ],
        crdt: false,
        sync: true,
        sync_scope: Some(SCOPE.into()),
        sync_limit,
        ..Default::default()
    };
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "sync-scope-scan".into(),
        version: "0.1.0".into(),
        entities: vec![
            ManifestEntity {
                name: "User".into(),
                fields: vec![field("email", "string")],
                ..Default::default()
            },
            ManifestEntity {
                name: "Member".into(),
                fields: vec![field("userId", "id(User)"), field("teamId", "string")],
                sync: false,
                ..Default::default()
            },
            scoped("Lead", None),
            scoped("Visit", Some(500)),
        ],
        policies: vec![
            allow_read("Lead"),
            allow_read("Visit"),
            allow_read("Member"),
        ],
        ..Default::default()
    }
}

/// 3,000 rows per scoped entity. The rep's scope keeps 40 of each: 10 they
/// own, and on their team t1, 15 seen in the last 30 days and 15 won; the
/// rest are on other teams, old, or someone else's. Returns the kept ids.
fn seed(rt: &Runtime) -> BTreeSet<String> {
    rt.insert(
        "Member",
        &serde_json::json!({"userId": "rep", "teamId": "t1"}),
    )
    .unwrap();
    let now = chrono::Utc::now();
    let ago = |days: i64| now - chrono::Duration::days(days);
    let mut expected = BTreeSet::new();
    for i in 0..3000u32 {
        let (owner, team, stage, seen) = match i % 100 {
            0 if i < 1000 => ("rep", "t2", "new", ago(400)),
            1 if i < 1500 => ("other", "t1", "new", ago(3)),
            2 if i < 1500 => ("other", "t1", "won", ago(400)),
            3 => ("other", "t1", "new", ago(31)),
            4 => ("other", "t2", "won", ago(1)),
            _ => ("other", "t3", "new", ago(1)),
        };
        let id = format!("{i:040x}");
        if owner == "rep" || (team == "t1" && (stage == "won" || seen > ago(30))) {
            expected.insert(id.clone());
        }
        let seen = seen.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        for entity in ENTITIES {
            rt.insert(
                entity,
                &serde_json::json!({
                    "id": id, "ownerId": owner, "teamId": team, "stage": stage, "seenAt": seen,
                }),
            )
            .unwrap();
        }
    }
    assert_eq!(expected.len(), 40);
    expected
}

fn free_port() -> u16 {
    for _ in 0..200 {
        let base = 20_000 + rand::random::<u16>() % 8_000;
        if (0..4).all(|o| pylon_runtime::listen::port_is_free(base + o)) {
            return base;
        }
    }
    panic!("no free port block");
}

fn http(port: u16, method: &str, path: &str, token: Option<&str>, body: &str) -> (u16, Value) {
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    // HTTP/1.0: the answer comes whole, not chunked.
    let req = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    stream.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok();
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

/// Starts the server on `rt` and signs in as the rep: (port, token).
fn serve(rt: Arc<Runtime>) -> (u16, String) {
    // SAFETY: set before the server thread starts; nothing else reads it.
    unsafe { std::env::set_var("PYLON_DEV_MODE", "1") };
    let port = free_port();
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            let (status, body) = http(
                port,
                "POST",
                "/api/auth/session",
                None,
                r#"{"user_id":"rep"}"#,
            );
            if status == 201 {
                return (port, body["token"].as_str().unwrap().to_string());
            }
        }
        assert!(Instant::now() < deadline, "server never came up");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The snapshot pull, page by page: the ids per scoped entity, and the
/// number of pages.
fn snapshot(port: u16, token: &str) -> (Vec<BTreeSet<String>>, usize) {
    let mut got = vec![BTreeSet::new(); ENTITIES.len()];
    let mut pages = 0;
    let mut path = "/api/sync/pull?since=0".to_string();
    loop {
        let (status, body) = http(port, "GET", &path, Some(token), "");
        assert_eq!(status, 200, "{body}");
        pages += 1;
        for change in body["changes"].as_array().unwrap() {
            if let Some(i) = ENTITIES.iter().position(|e| change["entity"] == *e) {
                got[i].insert(change["row_id"].as_str().unwrap().to_string());
            }
        }
        match body["snapshot_after"].as_str() {
            Some(after) if body["has_more"] == true => {
                path = format!("/api/sync/pull?since=0&snapshot_after={after}");
            }
            _ => break,
        }
    }
    (got, pages)
}

/// The reconcile fetch of `entity`, page by page.
fn reconcile(port: u16, token: &str, entity: &str) -> BTreeSet<String> {
    let mut got = BTreeSet::new();
    let mut path = format!("/api/entities/{entity}/cursor?limit=1000&sync=1");
    loop {
        let (status, body) = http(port, "GET", &path, Some(token), "");
        assert_eq!(status, 200, "{body}");
        for row in body["data"].as_array().unwrap() {
            got.insert(row["id"].as_str().unwrap().to_string());
        }
        match body["next_cursor"].as_str() {
            Some(cursor) if body["has_more"] == true => {
                path = format!("/api/entities/{entity}/cursor?limit=1000&sync=1&after={cursor}");
            }
            _ => break,
        }
    }
    got
}

#[test]
fn sqlite_replication_reads_only_the_rows_its_scope_keeps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scan.db");
    let rt = Arc::new(Runtime::open(path.to_str().unwrap(), manifest()).unwrap());
    let expected = seed(&rt);
    let (port, token) = serve(rt);

    // The snapshot pauses after reading this many rows. Reading only the
    // 80 in-scope rows takes a few pages; reading all 6,000 would take over
    // 100.
    pylon_router::set_snapshot_scan_budget(50);
    let (got, pages) = snapshot(port, &token);
    pylon_router::set_snapshot_scan_budget(0);
    for (i, entity) in ENTITIES.iter().enumerate() {
        assert_eq!(got[i], expected, "the {entity} snapshot");
    }
    assert!(
        pages <= 4,
        "the snapshot read out-of-scope rows: {pages} pages"
    );

    for entity in ENTITIES {
        assert_eq!(
            reconcile(port, &token, entity),
            expected,
            "the {entity} reconcile"
        );
    }
}

/// The test database's URL, pointed at the counting proxy.
fn proxied(url: &str) -> (pg_wire::WireCounter, String) {
    let config: postgres::Config = url.parse().expect("PYLON_TEST_PG_URL parses");
    let host = match &config.get_hosts()[0] {
        postgres::config::Host::Tcp(host) => host.clone(),
        #[cfg(unix)]
        other => panic!("the test needs a TCP host, not {other:?}"),
    };
    let port = config.get_ports().first().copied().unwrap_or(5432);
    let counter = pg_wire::WireCounter::start(format!("{host}:{port}"));
    let quote = |s: &str| format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"));
    let mut proxied = format!("host=127.0.0.1 port={} sslmode=disable", counter.port);
    if let Some(user) = config.get_user() {
        proxied.push_str(&format!(" user={}", quote(user)));
    }
    if let Some(password) = config.get_password() {
        proxied.push_str(&format!(
            " password={}",
            quote(&String::from_utf8_lossy(password))
        ));
    }
    if let Some(db) = config.get_dbname() {
        proxied.push_str(&format!(" dbname={}", quote(db)));
    }
    (counter, proxied)
}

/// Runs of `SELECT`s reading `entity`'s rows: (with the scope in the
/// query, without it).
fn page_queries(counts: &pg_wire::Counts, entity: &str) -> (usize, usize) {
    let reads = counts.run_matching(&format!("SELECT * FROM \"{entity}\""));
    let filtered = counts.run_matching(&format!("SELECT * FROM \"{entity}\" t WHERE"))
        - counts.run_matching(&format!(
            "SELECT * FROM \"{entity}\" t WHERE t.id < $1 ORDER"
        ))
        - counts.run_matching(&format!(
            "SELECT * FROM \"{entity}\" t WHERE t.id > $1 ORDER"
        ));
    (filtered, reads - filtered)
}

#[test]
fn postgres_replication_reads_only_the_rows_its_scope_keeps() {
    let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
        return;
    };
    let mut admin = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    admin
        .batch_execute("DROP TABLE IF EXISTS \"Lead\", \"Visit\", \"Member\", \"User\" CASCADE;")
        .unwrap();
    let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
    let plan = adapter.plan_from_live(&manifest()).unwrap();
    adapter.apply_plan(&plan).unwrap();

    let (wire, proxied_url) = proxied(&url);
    let rt = Arc::new(Runtime::open_postgres(&proxied_url, manifest()).unwrap());
    let expected = seed(&rt);
    let (port, token) = serve(rt);

    wire.reset();
    let (got, _) = snapshot(port, &token);
    let counts = wire.counts();
    for (i, entity) in ENTITIES.iter().enumerate() {
        assert_eq!(got[i], expected, "the {entity} snapshot");
        let (filtered, unfiltered) = page_queries(&counts, entity);
        assert_eq!(
            unfiltered, 0,
            "the {entity} snapshot read pages of every row"
        );
        assert!(
            (1..=2).contains(&filtered),
            "the {entity} snapshot ran {filtered} page queries"
        );
    }

    for entity in ENTITIES {
        wire.reset();
        assert_eq!(
            reconcile(port, &token, entity),
            expected,
            "the {entity} reconcile"
        );
        let (filtered, unfiltered) = page_queries(&wire.counts(), entity);
        assert_eq!(
            unfiltered, 0,
            "the {entity} reconcile read pages of every row"
        );
        assert!(
            (1..=2).contains(&filtered),
            "the {entity} reconcile ran {filtered} page queries"
        );
    }
}
