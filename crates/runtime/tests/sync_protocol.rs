//! Sync protocol correctness tests.
//!
//! These are the contract tests the sync engine needs to pass. Bugs in the
//! protocol silently corrupt client replicas; unit tests of individual
//! pieces don't catch the wiring issues that surface only when the full
//! pipeline runs (HTTP → change log → pull → cursor persistence). Each
//! test exercises one end-to-end scenario and asserts the invariant that
//! matters for local-first apps: the replica eventually matches the server.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestPolicy};
use pylon_runtime::Runtime;
use serde_json::Value;

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "sync-proto".into(),
        version: "0.1.0".into(),
        entities: vec![
            ManifestEntity {
                name: "Note".into(),
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
                        name: "body".into(),
                        field_type: "string".into(),
                        optional: true,
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
            },
            secret_entity(),
        ],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        // `Note` is intentionally public (these tests exercise the sync
        // pipeline, not auth — without a policy the secure default-deny
        // 403s the insert). `Secret` is statically deny-all: it stands in
        // for a large append-only server-only table (an audit log) that
        // must NEVER enter a client snapshot.
        policies: vec![
            ManifestPolicy {
                name: "note_public".into(),
                entity: Some("Note".into()),
                allow: "true".into(),
                ..Default::default()
            },
            ManifestPolicy {
                name: "secret_deny".into(),
                entity: Some("Secret".into()),
                allow_read: Some("false".into()),
                ..Default::default()
            },
        ],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

// A statically deny-all entity: one plain string field, `allowRead:"false"`
// via the `secret_deny` policy. Stands in for a large server-only table.
fn secret_entity() -> ManifestEntity {
    ManifestEntity {
        name: "Secret".into(),
        fields: vec![ManifestField {
            name: "data".into(),
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
        }],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        sync: true,
        ..Default::default()
    }
}

fn available_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(43_000);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        let ok = (0..4)
            .all(|off| std::net::TcpListener::bind(format!("127.0.0.1:{}", base + off)).is_ok());
        if ok {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn start_server(rt: Arc<Runtime>) -> u16 {
    let port = available_port();
    // Once per BINARY, not once per server. `cargo test` runs a binary's tests
    // on several threads by default, so a per-call `set_var` here is a data
    // race against every other thread reading the environment — which is why
    // set_var is unsafe. It surfaced as servers that never came up and tests
    // failing on `connect: ConnectionRefused`, on CI only.
    static DEV_MODE: std::sync::Once = std::sync::Once::new();
    DEV_MODE.call_once(|| {
        // SAFETY: exactly once, and before any server thread is spawned.
        unsafe { std::env::set_var("PYLON_DEV_MODE", "1") };
    });
    let rt2 = Arc::clone(&rt);
    // The server's own error is the only explanation for a bind
    // failure; dropping it leaves "never bound" with no cause.
    let boot_err: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let boot_err_thread = std::sync::Arc::clone(&boot_err);
    std::thread::spawn(move || {
        let r = pylon_runtime::server::start(rt2, port);
        if let Err(e) = r {
            *boot_err_thread.lock().unwrap() = Some(e.to_string());
        }
    });
    // 300 x 50ms = 15s. The old budget was 5s AND fell through
    // silently when it ran out, so a slow CI runner walked into a
    // bare `.expect("connect")` panic further down that looked like a
    // product bug. Fail here instead, naming the port.
    {
        let mut ready = false;
        for _ in 0..300 {
            if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            ready,
            "test server never bound {} within 15s (server error: {:?})",
            format!("127.0.0.1:{port}"),
            boot_err.lock().unwrap()
        );
    }
    port
}

fn http(
    port: u16,
    method: &str,
    path: &str,
    auth: Option<&str>,
    body: Option<&str>,
) -> (u16, String) {
    let body_str = body.unwrap_or("");
    let mut hdrs = format!(
        "Host: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body_str.len()
    );
    if let Some(t) = auth {
        hdrs.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    let req = format!("{method} {path} HTTP/1.1\r\n{hdrs}\r\n{body_str}");
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let (headers, raw_body) = match text.find("\r\n\r\n") {
        Some(i) => (text[..i].to_string(), text[i + 4..].to_string()),
        None => (String::new(), String::new()),
    };
    // De-chunk when the server used chunked transfer encoding — large
    // bodies (1000-row snapshot pages) arrive framed, and parsing the
    // raw text as JSON sees chunk-size lines interleaved with content.
    let body = if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut out = String::new();
        let mut rest = raw_body.as_str();
        loop {
            let Some(nl) = rest.find("\r\n") else { break };
            let size = usize::from_str_radix(rest[..nl].trim(), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            let start = nl + 2;
            let end = (start + size).min(rest.len());
            out.push_str(&rest[start..end]);
            rest = rest.get(end + 2..).unwrap_or("");
        }
        out
    } else {
        raw_body
    };
    (status, body)
}

fn mint_guest(port: u16) -> String {
    let (status, body) = http(port, "POST", "/api/auth/guest", None, None);
    assert_eq!(status, 201, "guest mint failed: {body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

fn pull(port: u16, token: &str, since: u64) -> (u16, Value) {
    let (status, body) = http(
        port,
        "GET",
        &format!("/api/sync/pull?since={since}"),
        Some(token),
        None,
    );
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    (status, v)
}

fn insert_note(port: u16, token: &str, title: &str) -> String {
    let (status, body) = http(
        port,
        "POST",
        "/api/entities/Note",
        Some(token),
        Some(&format!(r#"{{"title":"{title}","body":""}}"#)),
    );
    assert!(
        status == 200 || status == 201,
        "insert failed status={status} body={body}"
    );
    let v: Value = serde_json::from_str(&body).unwrap();
    v["id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// 1. Fresh client with empty server gets empty state, not errors
// ---------------------------------------------------------------------------
#[test]
fn fresh_pull_on_empty_server_returns_no_changes() {
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    let (status, resp) = pull(port, &token, 0);
    assert_eq!(status, 200);
    assert_eq!(resp["changes"].as_array().unwrap().len(), 0);
    assert_eq!(resp["cursor"]["last_seq"].as_u64().unwrap(), 0);
}

// ---------------------------------------------------------------------------
// 2. Insert → pull sees the change and advances cursor
// ---------------------------------------------------------------------------
#[test]
fn insert_then_pull_returns_change() {
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    let id = insert_note(port, &token, "hello");
    let (status, resp) = pull(port, &token, 0);
    assert_eq!(status, 200);
    let changes = resp["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["row_id"].as_str().unwrap(), id);
    assert_eq!(changes[0]["kind"].as_str().unwrap(), "insert");
    let new_cursor = resp["cursor"]["last_seq"].as_u64().unwrap();
    assert!(new_cursor > 0, "cursor must advance past 0");

    // Subsequent pull at advanced cursor returns nothing.
    let (_, resp2) = pull(port, &token, new_cursor);
    assert_eq!(resp2["changes"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// 3. Server restart with persisted DB: fresh client sees old rows
//    (validates the "seed change log from SQLite on startup" fix)
// ---------------------------------------------------------------------------
#[test]
fn server_restart_still_delivers_prior_rows_to_fresh_clients() {
    let tmpdir = tempfile::tempdir().unwrap();
    let db_path = tmpdir.path().join("sync.db");
    let db_str = db_path.to_str().unwrap();

    // First server lifetime: insert 3 rows, record cursor.
    let ids: Vec<String> = {
        let rt = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
        let port = start_server(rt);
        let token = mint_guest(port);
        let a = insert_note(port, &token, "one");
        let b = insert_note(port, &token, "two");
        let c = insert_note(port, &token, "three");
        vec![a, b, c]
    };
    // Let the first server's thread finish writes to disk. There's no
    // clean shutdown API; the runtime Arc goes out of scope, the HTTP
    // thread leaks, but SQLite writes are already committed.
    std::thread::sleep(Duration::from_millis(200));

    // Second lifetime: reopen the DB file. A fresh client should pull
    // all 3 rows via the seeded change log.
    let rt2 = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
    let port2 = start_server(rt2);
    let token2 = mint_guest(port2);
    let (status, resp) = pull(port2, &token2, 0);
    assert_eq!(status, 200);
    let changes = resp["changes"].as_array().unwrap();
    assert_eq!(
        changes.len(),
        3,
        "fresh pull after restart must surface all seeded rows; got: {:?}",
        changes
    );
    let got_ids: Vec<String> = changes
        .iter()
        .map(|c| c["row_id"].as_str().unwrap().to_string())
        .collect();
    for id in &ids {
        assert!(got_ids.contains(id), "missing {id}");
    }
}

// ---------------------------------------------------------------------------
// 4. Stale cursor from a previous server lifetime: get 410 (not silent empty)
// ---------------------------------------------------------------------------
#[test]
fn cursor_from_previous_lifetime_forces_resync() {
    let tmpdir = tempfile::tempdir().unwrap();
    let db_path = tmpdir.path().join("sync.db");
    let db_str = db_path.to_str().unwrap();

    // First lifetime: capture a cursor after some inserts.
    let first_cursor: u64 = {
        let rt = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
        let port = start_server(rt);
        let token = mint_guest(port);
        for i in 0..5 {
            insert_note(port, &token, &format!("row{i}"));
        }
        let (_, resp) = pull(port, &token, 0);
        resp["cursor"]["last_seq"].as_u64().unwrap()
    };
    assert!(first_cursor >= 5);
    std::thread::sleep(Duration::from_millis(200));

    // Second lifetime: the restart seeds `first_cursor` events again, so
    // the new seq counter also reaches that value. To force the "cursor
    // from an older lifetime" case we present a cursor beyond it.
    let rt2 = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
    let port2 = start_server(rt2);
    let token2 = mint_guest(port2);

    let stale = first_cursor + 1_000_000;
    let (status, resp) = pull(port2, &token2, stale);
    assert_eq!(
        status, 410,
        "stale cursor must force 410 RESYNC_REQUIRED, got {status}: {resp:?}"
    );
    assert_eq!(resp["error"]["code"].as_str().unwrap(), "RESYNC_REQUIRED");
}

// ---------------------------------------------------------------------------
// 5b. Session tokens survive server restart when runtime is file-backed.
//     Regression: SessionStore was in-memory by default; every dev-server
//     restart invalidated every browser token even though the app DB
//     carried on unchanged. Now persistence is automatic unless
//     PYLON_SESSION_IN_MEMORY=1.
// ---------------------------------------------------------------------------
#[test]
fn sessions_survive_server_restart_by_default() {
    let tmpdir = tempfile::tempdir().unwrap();
    let db_path = tmpdir.path().join("sync.db");
    let db_str = db_path.to_str().unwrap();
    // Make sure no stray env var forces us into the opt-out path.
    // Safety: tests run in-process; no other thread reads this var
    // between here and the server spawn.
    unsafe {
        std::env::remove_var("PYLON_SESSION_IN_MEMORY");
        std::env::remove_var("PYLON_SESSION_DB");
    }

    // First lifetime: mint a guest, confirm it resolves.
    let token = {
        let rt = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
        let port = start_server(rt);
        let t = mint_guest(port);
        let (s, body) = http(port, "GET", "/api/auth/me", Some(&t), None);
        assert_eq!(s, 200);
        let me: Value = serde_json::from_str(&body).unwrap();
        assert!(me["user_id"].is_string(), "guest session should resolve");
        t
    };
    std::thread::sleep(Duration::from_millis(200));

    // Second lifetime: reopen the same DB. The old token must still resolve
    // via the sibling sessions file created on first boot.
    let rt2 = Arc::new(Runtime::open(db_str, test_manifest()).unwrap());
    let port2 = start_server(rt2);
    let (s, body) = http(port2, "GET", "/api/auth/me", Some(&token), None);
    assert_eq!(s, 200);
    let me: Value = serde_json::from_str(&body).unwrap();
    assert!(
        me["user_id"].is_string(),
        "token minted under previous lifetime must still resolve: {body}"
    );
}

// ---------------------------------------------------------------------------
// 6. Cursor advances on empty (policy-filtered) pulls instead of sticking
// ---------------------------------------------------------------------------
#[test]
fn cursor_advances_even_when_response_is_empty() {
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    // Insert three rows and drain them.
    insert_note(port, &token, "a");
    insert_note(port, &token, "b");
    insert_note(port, &token, "c");
    let (_, resp) = pull(port, &token, 0);
    let cur = resp["cursor"]["last_seq"].as_u64().unwrap();
    assert!(cur >= 3);

    // Second pull at the advanced cursor: no new events, cursor should
    // echo back the same value (not 0). Clients rely on this — the
    // previous `changes.length > 0`-gated cursor assignment was the bug.
    let (_, resp2) = pull(port, &token, cur);
    assert_eq!(resp2["changes"].as_array().unwrap().len(), 0);
    assert_eq!(resp2["cursor"]["last_seq"].as_u64().unwrap(), cur);
    assert_eq!(resp2["has_more"].as_bool().unwrap(), false);
}

// ---------------------------------------------------------------------------
// A statically deny-all entity (allowRead:"false") is EXCLUDED from the
// snapshot — not merely per-row filtered. Regression for the 2026-06-03
// pylon-cloud egress storm: AuditEvent was member-readable + ever-growing,
// the snapshot walked the whole table on every connect (per-page live reads,
// no id ceiling) and never converged, so the client re-snapshotted since=0
// forever and never reached the WS phase. The snapshot now skips any entity
// `check_entity_scan` denies, so a deny-all table is never read or paginated.
// ---------------------------------------------------------------------------
#[test]
fn deny_all_entity_excluded_from_snapshot() {
    // Enable admin-token auth so we can prove the deny-all entity is
    // excluded even for an ADMIN (the pylon-cloud case — the dashboard owner
    // is an admin, so the policy deny was bypassed and AuditEvent looped).
    // Inert for other tests; they never present this token. SAFETY: same
    // single-threaded-setup rationale as start_server's PYLON_DEV_MODE.
    const ADMIN_TOKEN: &str = "sync-proto-admin-token";
    // Once per binary — see start_server. A per-call set_var races the other
    // test threads reading the environment.
    static ADMIN_ENV: std::sync::Once = std::sync::Once::new();
    ADMIN_ENV.call_once(|| unsafe {
        std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN_TOKEN);
    });

    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let port = start_server(Arc::clone(&rt));
    let token = mint_guest(port);

    // Readable Note via HTTP (policy-allowed; advances the change-log seq).
    insert_note(port, &token, "visible");
    // Seed the deny-all Secret entity directly — the inherent insert bypasses
    // the HTTP policy gate, like a server-side write. 1500 rows: more than
    // SNAPSHOT_BATCH_LIMIT, so a pre-fix snapshot paged through all of them
    // only to drop every one at the per-row fence.
    for i in 0..1500 {
        rt.insert("Secret", &serde_json::json!({ "data": format!("s{i}") }))
            .expect("seed secret");
    }

    let (status, resp) = pull(port, &token, 0);
    assert_eq!(status, 200, "snapshot pull must succeed: {resp}");
    let changes = resp["changes"].as_array().unwrap();
    assert!(
        changes.iter().any(|c| c["entity"] == "Note"),
        "readable Note must be in the snapshot: {resp}"
    );
    assert!(
        changes.iter().all(|c| c["entity"] != "Secret"),
        "deny-all Secret must be entirely absent — not one of 1500 rows leaks: {resp}"
    );
    // Snapshot CONVERGES: has_more=false on the same page (Secret never
    // paginated), cursor advances off 0 → the client graduates to the WS
    // live phase instead of re-snapshotting since=0.
    assert_eq!(
        resp["has_more"].as_bool().unwrap_or(false),
        false,
        "snapshot must converge in one page: {resp}"
    );
    assert!(
        resp["cursor"]["last_seq"].as_u64().unwrap() > 0,
        "cursor must advance off 0: {resp}"
    );

    // The case that actually bit pylon-cloud: an ADMIN. Admin bypasses the
    // per-row read fence, but a snapshot is BULK replication — a deny-all
    // table must never stream wholesale into ANY replica, admin included.
    // Pre-fix the admin saw all 1500 Secret rows (policy bypassed) and the
    // ever-growing table looped. `is_read_statically_denied` has no admin
    // bypass, so it stays excluded.
    let (admin_status, admin_resp) = pull(port, ADMIN_TOKEN, 0);
    assert_eq!(admin_status, 200, "admin pull must succeed: {admin_resp}");
    let admin_changes = admin_resp["changes"].as_array().unwrap();
    assert!(
        admin_changes.iter().all(|c| c["entity"] != "Secret"),
        "deny-all Secret must be excluded even for ADMIN: {admin_resp}"
    );
    assert_eq!(
        admin_resp["has_more"].as_bool().unwrap_or(false),
        false,
        "admin snapshot must converge too: {admin_resp}"
    );
}

// ---------------------------------------------------------------------------
// A `sync: false` entity is excluded from BOTH the snapshot and the change-log
// delta — even with a PUBLIC read policy. This is the knob that lets a large,
// server-queried catalog (the store's 10k-row Product table) stay out of every
// client replica while remaining directly readable via /api/entities + search.
// Distinct from `deny_all_entity_excluded_from_snapshot`: there a deny-all
// policy drives the exclusion; here the rows are fully readable and only the
// `sync` flag keeps them out of bulk replication. Without the flag the public
// catalog would flood the snapshot AND re-flood via the delta tail forever.
// ---------------------------------------------------------------------------
#[test]
fn sync_false_entity_excluded_from_snapshot_and_delta() {
    let mut manifest = test_manifest();
    manifest.entities.push(ManifestEntity {
        name: "Catalog".into(),
        fields: vec![ManifestField {
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
        }],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        // The entity under test: public to read, but never bulk-replicated.
        sync: false,
        ..Default::default()
    });
    manifest.policies.push(ManifestPolicy {
        name: "catalog_public".into(),
        entity: Some("Catalog".into()),
        allow: "true".into(),
        ..Default::default()
    });

    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    // Seed a synced Note and several Catalog rows through the HTTP write path.
    insert_note(port, &token, "synced-note");
    for i in 0..5 {
        let (s, b) = http(
            port,
            "POST",
            "/api/entities/Catalog",
            Some(&token),
            Some(&format!(r#"{{"title":"c{i}"}}"#)),
        );
        assert!(s == 200 || s == 201, "catalog insert: status={s} {b}");
    }

    // The catalog isn't hidden — direct reads still work (policy allows). It's
    // only excluded from bulk replication, not from the API surface.
    let (ls, lb) = http(port, "GET", "/api/entities/Catalog", Some(&token), None);
    assert_eq!(
        ls, 200,
        "sync:false catalog must stay directly readable: {lb}"
    );
    assert!(
        lb.contains("c0") && lb.contains("c4"),
        "direct catalog read must return the rows: {lb}"
    );

    // 1. SNAPSHOT (since=0): synced Note present, Catalog entirely absent, converges.
    let (status, resp) = pull(port, &token, 0);
    assert_eq!(status, 200, "snapshot pull: {resp}");
    let changes = resp["changes"].as_array().unwrap();
    assert!(
        changes.iter().any(|c| c["entity"] == "Note"),
        "synced Note must be in the snapshot: {resp}"
    );
    assert!(
        changes.iter().all(|c| c["entity"] != "Catalog"),
        "sync:false Catalog must be absent from the snapshot — not one of 5 rows leaks: {resp}"
    );
    assert_eq!(
        resp["has_more"].as_bool().unwrap_or(false),
        false,
        "snapshot must converge (Catalog never paginated): {resp}"
    );
    let cursor = resp["cursor"]["last_seq"].as_u64().unwrap();
    assert!(cursor > 0, "cursor must advance off 0: {resp}");

    // 2. DELTA (since=cursor): a post-snapshot Catalog write must NOT stream
    // (else the tail re-floods what the snapshot deliberately skipped); a
    // post-snapshot Note write MUST stream.
    let (cs, cb) = http(
        port,
        "POST",
        "/api/entities/Catalog",
        Some(&token),
        Some(r#"{"title":"post-snapshot-catalog"}"#),
    );
    assert!(cs == 200 || cs == 201, "post-snapshot catalog insert: {cb}");
    let new_note = insert_note(port, &token, "post-snapshot-note");

    let (ds, dresp) = pull(port, &token, cursor);
    assert_eq!(ds, 200, "delta pull: {dresp}");
    let dchanges = dresp["changes"].as_array().unwrap();
    assert!(
        dchanges
            .iter()
            .any(|c| c["row_id"].as_str() == Some(new_note.as_str())),
        "post-snapshot Note delta must stream: {dresp}"
    );
    assert!(
        dchanges.iter().all(|c| c["entity"] != "Catalog"),
        "sync:false Catalog delta must NOT stream (would re-flood the replica): {dresp}"
    );
}

// ---------------------------------------------------------------------------
// Security: /api/sync/push must NOT let a non-admin write framework-internal
// `_`-prefixed entities. The policy gate allows them (no app policy ⇒ allowed
// for underscore entities), trusting the route edge to gate — the entity REST
// surface does, and the push surface must too. Pre-fix a guest could insert/
// update/delete `_Connection`, `_PylonJobs`, etc. through push. The gate is
// per-op: a rejected `_`-op must not block legit ops in the same batch.
// ---------------------------------------------------------------------------
#[test]
fn push_rejects_underscore_entities_for_non_admin() {
    // A REGISTERED `_`-entity (framework-internal, no app policy) — stands in
    // for `_Connection`/`_PylonJobs` etc. on a real deployment. Registered, so
    // the mutation pipeline does NOT reject it as "unknown entity"; the policy
    // gate ALLOWS it (underscore bypass). The route-edge gate is the only thing
    // standing between a guest and writing framework state.
    let mut manifest = test_manifest();
    manifest.entities.push(ManifestEntity {
        name: "_Internal".into(),
        fields: vec![ManifestField {
            name: "val".into(),
            field_type: "string".into(),
            optional: true,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: None,
            enum_values: None,
            encrypted: false,
            sync_omit: false,
        }],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        sync: true,
        ..Default::default()
    });
    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port); // non-admin

    // Without the gate this insert APPLIES (registered entity + policy bypass)
    // — the actual privilege hole. The legit Note insert in the same batch
    // proves the gate is per-op, not a whole-request rejection.
    let body = r#"{"changes":[
        {"op_id":"op-evil","entity":"_Internal","row_id":"i1","kind":"insert","data":{"val":"x"}},
        {"op_id":"op-ok","entity":"Note","row_id":"n1","kind":"insert","data":{"title":"hi","body":""}}
    ]}"#;
    let (status, resp_body) = http(port, "POST", "/api/sync/push", Some(&token), Some(body));
    assert_eq!(status, 200, "push request itself returns 200: {resp_body}");
    let resp: Value = serde_json::from_str(&resp_body).unwrap();
    let results = resp["results"].as_array().expect("per-op results");
    let evil = results
        .iter()
        .find(|r| r["op_id"] == "op-evil")
        .expect("evil op result");
    assert_eq!(
        evil["status"], "error",
        "underscore-entity write by a non-admin must be rejected: {resp_body}"
    );
    assert_eq!(
        evil["error"]["code"], "NOT_FOUND",
        "rejection must not confirm the table exists: {resp_body}"
    );
    // Per-op gate: the legit Note insert in the SAME batch still applies.
    let ok = results
        .iter()
        .find(|r| r["op_id"] == "op-ok")
        .expect("ok op result");
    assert_eq!(
        ok["status"], "applied",
        "legit op must still apply: {resp_body}"
    );

    // And it really didn't write: no `_Internal` change leaked into the log.
    let (_, pull_resp) = pull(port, &token, 0);
    let changes = pull_resp["changes"].as_array().unwrap();
    assert!(
        changes.iter().all(|c| c["entity"] != "_Internal"),
        "rejected underscore write must not have hit the store: {pull_resp}"
    );
}

// ---------------------------------------------------------------------------
// Security: readonly fields — the owner stamped by `field.owner()` and
// identity columns like orgId/tenantId/createdBy — are client-immutable on
// every client write surface, including /api/sync/push (the path the SDK
// steers local-first apps to). A push Update touching a readonly field by a
// non-admin is rejected (READONLY_FIELD) so it can't reassign ownership or
// flip a tenant scope (IDOR). Per-op: a legit non-readonly edit in the same
// batch still applies — the gate must not over-block.
// ---------------------------------------------------------------------------
#[test]
fn push_update_cannot_flip_readonly_field_for_non_admin() {
    fn field(name: &str, readonly: bool, optional: bool) -> ManifestField {
        ManifestField {
            name: name.into(),
            field_type: "string".into(),
            optional,
            unique: false,
            crdt: None,
            server_only: false,
            readonly,
            default: None,
            enum_values: None,
            encrypted: false,
            sync_omit: false,
        }
    }
    let mut manifest = test_manifest();
    manifest.entities.push(ManifestEntity {
        name: "Owned".into(),
        // `ownerId` stands in for a field.owner() stamp: readonly so a client
        // can't reassign it after insert.
        fields: vec![field("title", false, false), field("ownerId", true, true)],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: true,
        sync: true,
        ..Default::default()
    });
    manifest.policies.push(ManifestPolicy {
        name: "owned_public".into(),
        entity: Some("Owned".into()),
        allow: "true".into(),
        ..Default::default()
    });
    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port); // non-admin

    // Insert a row the caller "owns". Insert is the owner-stamp's path and is
    // not gated here; we just need a row carrying a readonly ownerId.
    let insert = r#"{"changes":[
        {"op_id":"op-ins","entity":"Owned","row_id":"r1","kind":"insert","data":{"title":"mine","ownerId":"me"}}
    ]}"#;
    let (s, b) = http(port, "POST", "/api/sync/push", Some(&token), Some(insert));
    assert_eq!(s, 200, "insert push: {b}");
    let r: Value = serde_json::from_str(&b).unwrap();
    assert_eq!(
        r["results"][0]["status"], "applied",
        "insert must apply: {b}"
    );
    // The server assigns the canonical row id (the wire row_id "r1" isn't a
    // valid generated id, so it mints its own) — target that for the update.
    let row_id = r["results"][0]["row_id"].as_str().expect("assigned row id");

    // The attack (flip ownerId) + a legit title edit in the SAME batch.
    let update = format!(
        r#"{{"changes":[
        {{"op_id":"op-evil","entity":"Owned","row_id":"{row_id}","kind":"update","data":{{"ownerId":"victim"}}}},
        {{"op_id":"op-ok","entity":"Owned","row_id":"{row_id}","kind":"update","data":{{"title":"renamed"}}}}
    ]}}"#
    );
    let (s2, b2) = http(port, "POST", "/api/sync/push", Some(&token), Some(&update));
    assert_eq!(s2, 200, "update push returns 200: {b2}");
    let r2: Value = serde_json::from_str(&b2).unwrap();
    let results = r2["results"].as_array().expect("results");
    let evil = results
        .iter()
        .find(|x| x["op_id"] == "op-evil")
        .expect("evil result");
    assert_eq!(
        evil["status"], "error",
        "readonly ownerId flip must be rejected: {b2}"
    );
    assert_eq!(
        evil["error"]["code"], "READONLY_FIELD",
        "rejection must be READONLY_FIELD: {b2}"
    );
    let ok = results
        .iter()
        .find(|x| x["op_id"] == "op-ok")
        .expect("ok result");
    assert_eq!(
        ok["status"], "applied",
        "legit non-readonly title edit must still apply: {b2}"
    );

    // Ownership really didn't change; the legit edit did.
    let (_, pull_resp) = pull(port, &token, 0);
    let owned = pull_resp["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["entity"] == "Owned")
        .expect("Owned row in snapshot")
        .clone();
    assert_eq!(
        owned["data"]["ownerId"], "me",
        "ownerId must be unchanged after the rejected flip: {pull_resp}"
    );
    assert_eq!(
        owned["data"]["title"], "renamed",
        "legit title edit must have applied: {pull_resp}"
    );
}

// ---------------------------------------------------------------------------
// Robustness: a snapshot pull must bound rows SCANNED per request, not just
// rows emitted. A data-dependent sparse policy (auth.userId == data.ownerId on
// a large shared table where the caller owns few rows) passes the entity-scan
// gate but drops most rows at the per-row read fence — so without a scan budget
// the loop walks the entire table in one request (DB-egress / request-timeout
// storm, then a never-converging since=0 re-snapshot). With the budget set
// below the row count the pull paginates by scan progress and still converges.
// ---------------------------------------------------------------------------
#[test]
fn snapshot_pull_bounds_rows_scanned_for_sparse_policy() {
    fn plain(name: &str) -> ManifestField {
        ManifestField {
            name: name.into(),
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
        }
    }
    let mut manifest = test_manifest();
    manifest.entities.push(ManifestEntity {
        name: "Shared".into(),
        fields: vec![plain("ownerId"), plain("data")],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: true,
        sync: true,
        ..Default::default()
    });
    // Data-dependent per-row policy: not statically deniable, so the entity is
    // scanned and filtered row-by-row — the path that scanned unboundedly.
    manifest.policies.push(ManifestPolicy {
        name: "shared_owner_read".into(),
        entity: Some("Shared".into()),
        allow_read: Some("auth.userId == data.ownerId".into()),
        ..Default::default()
    });

    // Budget below the seeded row count forces the scan to paginate. Set via
    // the atomic override, NOT the environment: `set_var` from a test thread
    // races every other thread reading the environment (hence unsafe), and on
    // CI that aborted the whole binary — every remaining test in this file then
    // failed on `connect: ConnectionRefused`.
    pylon_router::set_snapshot_scan_budget(50);
    // Restored below so a 50-row budget cannot leak into the test that seeds
    // 1,500 rows and expects a full snapshot.

    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    // 60 rows owned by someone else — the guest matches none of them.
    for i in 0..60 {
        rt.insert(
            "Shared",
            &serde_json::json!({ "ownerId": "someone-else", "data": format!("d{i}") }),
        )
        .expect("seed shared");
    }
    let port = start_server(rt);
    let token = mint_guest(port); // userId != "someone-else"

    // Page 1: pre-fix this walks all 60 in one request → has_more=false. With
    // the budget it stops at 50 scanned and returns a continuation token.
    let (s1, b1) = http(port, "GET", "/api/sync/pull?since=0", Some(&token), None);
    assert_eq!(s1, 200, "page1: {b1}");
    let r1: Value = serde_json::from_str(&b1).unwrap();
    assert_eq!(
        r1["has_more"], true,
        "scan budget must paginate a sparse scan, not walk the whole table: {b1}"
    );
    assert_eq!(
        r1["cursor"]["last_seq"], 0,
        "cursor stays 0 while paginating: {b1}"
    );
    let token_after = r1["snapshot_after"]
        .as_str()
        .expect("continuation token on page 1")
        .to_string();

    // Page 2: resume — scans the remaining 10 and converges.
    let (s2, b2) = http(
        port,
        "GET",
        &format!("/api/sync/pull?since=0&snapshot_after={token_after}"),
        Some(&token),
        None,
    );
    assert_eq!(s2, 200, "page2: {b2}");
    let r2: Value = serde_json::from_str(&b2).unwrap();
    assert_eq!(
        r2["has_more"], false,
        "snapshot must converge once the table is fully scanned: {b2}"
    );

    // Default restored: a leaked 50-row budget would truncate the snapshot in
    // the test that seeds 1,500 rows.
    pylon_router::set_snapshot_scan_budget(0);
}

// ---------------------------------------------------------------------------
// Replication scope (`sync_scope` / `sync_limit`)
// ---------------------------------------------------------------------------
//
// A scope bounds WHICH rows of a synced entity reach a replica, so an entity
// that grows without bound can stay live instead of being forced to
// `sync: false`. These drive the real HTTP surface — the engine-level
// semantics are unit-tested in pylon-policy; what matters here is that the
// snapshot, the cursor bootstrap and the delta tail all honour it, since a
// gap in any one of them silently ships the whole table.

/// `Note` scoped to rows whose `body` marks them as belonging to the caller's
/// bucket. `body` is used as the scope key purely because the fixture entity
/// already has it — the predicate shape is what's under test.
fn scoped_manifest(scope: &str, limit: Option<usize>) -> AppManifest {
    let mut m = test_manifest();
    for e in m.entities.iter_mut() {
        if e.name == "Note" {
            e.sync_scope = Some(scope.to_string());
            e.sync_limit = limit;
        }
    }
    m
}

#[test]
fn snapshot_replicates_only_rows_inside_the_scope() {
    let rt = Arc::new(Runtime::in_memory(scoped_manifest("data.body == \"keep\"", None)).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    // Two rows in scope, one out.
    for (title, body) in [("a", "keep"), ("b", "drop"), ("c", "keep")] {
        let (status, resp) = http(
            port,
            "POST",
            "/api/entities/Note",
            Some(&token),
            Some(&format!(r#"{{"title":"{title}","body":"{body}"}}"#)),
        );
        assert!(status == 200 || status == 201, "insert failed: {resp}");
    }

    let (status, resp) = pull(port, &token, 0);
    assert_eq!(status, 200);
    let titles: Vec<&str> = resp["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["entity"] == "Note")
        .map(|c| c["data"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["a", "c"], "snapshot ignored the scope: {resp}");
}

#[test]
fn an_unscoped_entity_is_untouched_by_the_feature() {
    // The default has to stay exactly as it was — this is the regression that
    // would hit every existing app.
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    insert_note(port, &token, "a");
    insert_note(port, &token, "b");

    let (_, resp) = pull(port, &token, 0);
    let n = resp["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["entity"] == "Note")
        .count();
    assert_eq!(n, 2, "unscoped entity lost rows: {resp}");
}

#[test]
fn a_row_leaving_scope_is_pushed_as_a_delete() {
    // The subtle one. If an update that moves a row OUT of scope were simply
    // dropped, every replica would keep a stale copy forever — visible, never
    // updated again. It has to arrive as a delete so the client evicts it.
    let rt = Arc::new(Runtime::in_memory(scoped_manifest("data.body == \"keep\"", None)).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    let (status, body) = http(
        port,
        "POST",
        "/api/entities/Note",
        Some(&token),
        Some(r#"{"title":"a","body":"keep"}"#),
    );
    assert!(status == 200 || status == 201, "insert failed: {body}");
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (_, snap) = pull(port, &token, 0);
    let since = snap["cursor"]["last_seq"].as_u64().unwrap();

    // Move it out of scope.
    let (status, body) = http(
        port,
        "PATCH",
        &format!("/api/entities/Note/{id}"),
        Some(&token),
        Some(r#"{"body":"drop"}"#),
    );
    assert!(status == 200, "update failed status={status} body={body}");

    let (_, resp) = pull(port, &token, since);
    let ev = resp["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["row_id"] == id.as_str())
        .unwrap_or_else(|| panic!("no event for the row that left scope: {resp}"));
    assert_eq!(
        ev["kind"].as_str().unwrap(),
        "delete",
        "row left scope but was not evicted: {resp}"
    );
}

#[test]
fn sync_limit_caps_rows_replicated_per_entity() {
    let rt = Arc::new(Runtime::in_memory(scoped_manifest("true", Some(2))).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    for i in 0..6 {
        insert_note(port, &token, &format!("n{i}"));
    }

    let (_, resp) = pull(port, &token, 0);
    let n = resp["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["entity"] == "Note")
        .count();
    assert_eq!(n, 2, "sync_limit did not cap the snapshot: {resp}");
}

#[test]
fn sync_limit_keeps_the_NEWEST_rows() {
    // The cap is aimed at time-series tables, and every other scan walks ids
    // ASCENDING — so the first cut of this shipped the OLDEST rows, which for
    // a dashboard showing "the last 30 days" is exactly the rows nobody
    // wants. A capped entity must replicate the tail of the table.
    let rt = Arc::new(Runtime::in_memory(scoped_manifest("true", Some(3))).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    for i in 0..10 {
        insert_note(port, &token, &format!("n{i}"));
    }

    let (_, resp) = pull(port, &token, 0);
    let titles: Vec<&str> = resp["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["entity"] == "Note")
        .map(|c| c["data"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        vec!["n7", "n8", "n9"],
        "capped entity replicated the head of the table, not the tail: {resp}"
    );
}

#[test]
fn a_cap_counts_rows_THIS_CALLER_can_see() {
    // The bug this nearly shipped with: `list_last(cap)` as a single fetch
    // takes the newest `cap` rows across EVERY tenant and only then applies
    // the read policy — so on a busy table a quiet tenant gets nothing. The
    // cap has to count VISIBLE rows, which means paging backwards until it
    // has them.
    //
    // Modeled with a scope standing in for the tenant fence: only "mine"
    // rows are visible, and they are the OLDEST — so a single tail fetch of
    // 2 would return two "theirs" rows and replicate zero of mine.
    let rt =
        Arc::new(Runtime::in_memory(scoped_manifest("data.body == \"mine\"", Some(2))).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    for title in ["m0", "m1"] {
        let (status, body) = http(
            port,
            "POST",
            "/api/entities/Note",
            Some(&token),
            Some(&format!(r#"{{"title":"{title}","body":"mine"}}"#)),
        );
        assert!(status == 200 || status == 201, "insert failed: {body}");
    }
    // 50 newer rows belonging to somebody else.
    for i in 0..50 {
        let (status, body) = http(
            port,
            "POST",
            "/api/entities/Note",
            Some(&token),
            Some(&format!(r#"{{"title":"t{i}","body":"theirs"}}"#)),
        );
        assert!(status == 200 || status == 201, "insert failed: {body}");
    }

    let (_, resp) = pull(port, &token, 0);
    let titles: Vec<&str> = resp["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["entity"] == "Note")
        .map(|c| c["data"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        vec!["m0", "m1"],
        "the cap counted rows the caller cannot see, starving them: {resp}"
    );
}

#[test]
fn the_cursor_bootstrap_honours_the_scope_but_a_direct_read_does_not() {
    // `sync: false` promises direct reads are unchanged, and a scope makes the
    // same promise: only the sync engine's REPLICATION fetch (`sync=1`) is
    // scoped. An app paginating the table — an archive view, an admin report —
    // must still see everything its policy allows.
    let rt = Arc::new(Runtime::in_memory(scoped_manifest("data.body == \"keep\"", None)).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    for (title, body) in [("a", "keep"), ("b", "drop")] {
        let (status, resp) = http(
            port,
            "POST",
            "/api/entities/Note",
            Some(&token),
            Some(&format!(r#"{{"title":"{title}","body":"{body}"}}"#)),
        );
        assert!(status == 200 || status == 201, "insert failed: {resp}");
    }

    let count = |qs: &str| -> usize {
        let (status, body) = http(
            port,
            "GET",
            &format!("/api/entities/Note/cursor?limit=100{qs}"),
            Some(&token),
            None,
        );
        assert_eq!(status, 200, "cursor failed: {body}");
        serde_json::from_str::<Value>(&body).unwrap()["data"]
            .as_array()
            .unwrap()
            .len()
    };

    assert_eq!(count("&sync=1"), 1, "replication fetch ignored the scope");
    assert_eq!(count(""), 2, "a direct read was wrongly scoped");
}

// ---------------------------------------------------------------------------
// Delta pull: page refill + far-behind cutover
// ---------------------------------------------------------------------------

/// A delta page must be refilled AFTER the policy fence. The change log
/// pages raw events, and a caller behind a span of events it can't see
/// (another tenant's writes, a deny-all audit table) used to get a
/// near-empty page per round trip — 1100 invisible events cost 11 full
/// round trips to deliver 3 visible ones. The refill loop keeps
/// scanning until a real page accumulates, so this converges in ONE
/// request.
#[test]
fn delta_pull_refills_pages_dropped_by_the_policy_fence() {
    fn plain(name: &str) -> ManifestField {
        ManifestField {
            name: name.into(),
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
        }
    }
    let mut manifest = test_manifest();
    manifest.entities.push(ManifestEntity {
        name: "Shared".into(),
        fields: vec![plain("ownerId"), plain("data")],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        sync: true,
        ..Default::default()
    });
    // Writable by anyone, readable only by the row's owner — so the
    // caller's own change feed can carry a long span of events the
    // policy fence drops.
    manifest.policies.push(ManifestPolicy {
        name: "shared_write_open".into(),
        entity: Some("Shared".into()),
        allow: "true".into(),
        allow_read: Some("auth.userId == data.ownerId".into()),
        ..Default::default()
    });
    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    let port = start_server(Arc::clone(&rt));
    let token = mint_guest(port);

    // Anchor write BEFORE the bootstrap pull: over an empty log the
    // snapshot hands back cursor=0, and a since=0 "delta" would route
    // straight back to the snapshot path instead of the code under test.
    insert_note(port, &token, "anchor");
    let (s0, r0) = pull(port, &token, 0);
    assert_eq!(s0, 200);
    let cursor = r0["cursor"]["last_seq"].as_u64().unwrap();
    assert!(
        cursor > 0,
        "anchor write must move the snapshot cursor off 0"
    );

    // 1100 events the puller can never see (owned by someone else) —
    // more than one raw change-log page — pushed as one batch through
    // the real wire path (only the mutation pipeline appends to the
    // change log), then 3 visible notes.
    let ops: Vec<String> = (0..1100)
        .map(|i| {
            format!(
                r#"{{"op_id":"op{i}","entity":"Shared","row_id":"s{i}","kind":"insert","data":{{"ownerId":"someone-else","data":"d{i}"}}}}"#
            )
        })
        .collect();
    let body = format!(r#"{{"changes":[{}]}}"#, ops.join(","));
    let (sp, bp) = http(port, "POST", "/api/sync/push", Some(&token), Some(&body));
    assert_eq!(sp, 200, "batch push failed: {bp}");
    for title in ["v1", "v2", "v3"] {
        insert_note(port, &token, title);
    }

    let (s1, r1) = pull(port, &token, cursor);
    assert_eq!(s1, 200, "delta pull failed: {r1}");
    let changes = r1["changes"].as_array().unwrap();
    assert_eq!(
        changes.len(),
        3,
        "one refilled request must deliver every visible event across the invisible span: {r1}"
    );
    assert!(
        changes.iter().all(|c| c["entity"] == "Note"),
        "no Secret event may survive the fence: {r1}"
    );
    assert_eq!(
        r1["has_more"], false,
        "the refilled page covered the whole tail, so has_more must be false: {r1}"
    );
    assert!(
        r1["cursor"]["last_seq"].as_u64().unwrap() > cursor + 1100,
        "cursor must advance past the filtered span so it is never re-scanned: {r1}"
    );
}

/// Far-behind cutover: when the cursor gap dwarfs what a snapshot would
/// send (every intermediate change coalesced, deleted rows absent), the
/// server forces the snapshot path with the same 410 the retention
/// window uses, instead of replaying an arbitrarily long delta.
#[test]
fn delta_pull_far_behind_cuts_over_to_snapshot_resync() {
    // Atomic override, not env — set_var races other test threads.
    pylon_router::set_delta_resync_threshold(50);

    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    // Anchor write so the bootstrap cursor is non-zero (a since=0 pull
    // routes to the snapshot path, not the delta path under test).
    insert_note(port, &token, "anchor");
    let (s0, r0) = pull(port, &token, 0);
    assert_eq!(s0, 200);
    let cursor = r0["cursor"]["last_seq"].as_u64().unwrap();
    assert!(
        cursor > 0,
        "anchor write must move the snapshot cursor off 0"
    );

    // 30 inserts + 30 deletes = a 60-event gap over a table that ends
    // EMPTY: the delta replays 60 events, the snapshot sends none.
    let mut ids = Vec::new();
    for i in 0..30 {
        ids.push(insert_note(port, &token, &format!("churn{i}")));
    }
    for id in &ids {
        let (status, resp) = http(
            port,
            "DELETE",
            &format!("/api/entities/Note/{id}"),
            Some(&token),
            None,
        );
        assert!(status == 200 || status == 204, "delete failed: {resp}");
    }

    let (s1, r1) = pull(port, &token, cursor);
    assert_eq!(
        s1, 410,
        "a gap of 60 events over 0 surviving rows must cut over to a snapshot resync: {r1}"
    );
    assert_eq!(r1["error"]["code"], "RESYNC_REQUIRED", "{r1}");

    // The client's 410 recovery path: reset to since=0 and re-snapshot.
    // It must land at the tip with no Note rows.
    let (s2, r2) = pull(port, &token, 0);
    assert_eq!(s2, 200, "recovery snapshot failed: {r2}");
    let notes = r2["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["entity"] == "Note")
        .count();
    assert_eq!(
        notes, 1,
        "recovery snapshot must hold only the surviving anchor note — deleted churn rows must not reappear: {r2}"
    );
    assert_eq!(r2["has_more"], false, "{r2}");

    pylon_router::set_delta_resync_threshold(0);
}

/// Rolling time window via `sync: { where: 'data.<ts> >= ago("30d")' }`.
/// The window is what keeps an append-only table's replica (and its
/// catch-up cost) bounded as the table grows: the snapshot ships only
/// in-window rows, and the reconcile sweep (`?sync=1`) evicts rows as
/// they age out. `body` doubles as the timestamp field because the
/// fixture entity already has it — the predicate shape is under test.
#[test]
fn snapshot_honors_a_rolling_time_window_scope() {
    let rt =
        Arc::new(Runtime::in_memory(scoped_manifest("data.body >= ago(\"30d\")", None)).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let yesterday = pylon_kernel::util::epoch_to_iso(now_secs - 86_400);
    let last_year = pylon_kernel::util::epoch_to_iso(now_secs - 365 * 86_400);

    for (title, ts) in [("recent", &yesterday), ("stale", &last_year)] {
        let (status, resp) = http(
            port,
            "POST",
            "/api/entities/Note",
            Some(&token),
            Some(&format!(r#"{{"title":"{title}","body":"{ts}"}}"#)),
        );
        assert!(status == 200 || status == 201, "insert failed: {resp}");
    }

    let (s, r) = pull(port, &token, 0);
    assert_eq!(s, 200, "snapshot failed: {r}");
    let notes: Vec<&Value> = r["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["entity"] == "Note")
        .collect();
    assert_eq!(notes.len(), 1, "only the in-window row may replicate: {r}");
    assert_eq!(notes[0]["data"]["title"], "recent", "{r}");

    // The replication cursor fetch (`?sync=1`) — the reconcile path that
    // evicts aged-out rows — honors the same window; a direct read does not.
    let count = |qs: &str| -> usize {
        let (status, body) = http(
            port,
            "GET",
            &format!("/api/entities/Note/cursor?limit=100{qs}"),
            Some(&token),
            None,
        );
        assert_eq!(status, 200, "cursor failed: {body}");
        serde_json::from_str::<Value>(&body).unwrap()["data"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(count("&sync=1"), 1, "replication fetch ignored the window");
    assert_eq!(count(""), 2, "a direct read was wrongly windowed");
}

// ---------------------------------------------------------------------------
// No-op update suppression
// ---------------------------------------------------------------------------

/// An update that leaves the row byte-identical must NOT append a change
/// event or advance the log — cron-style snapshot rewrites ("recompute
/// this stat table every minute") otherwise flood the change log with
/// events describing a table that never changed, and every returning
/// client replays them. CRDT entities are exempt: identical row JSON
/// can still carry a Loro version-vector advance peers need.
#[test]
fn noop_update_appends_no_change_event() {
    let mut manifest = test_manifest();
    manifest.entities.push(ManifestEntity {
        name: "Plain".into(),
        fields: vec![ManifestField {
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
        }],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        sync: true,
        ..Default::default()
    });
    manifest.policies.push(ManifestPolicy {
        name: "plain_public".into(),
        entity: Some("Plain".into()),
        allow: "true".into(),
        ..Default::default()
    });
    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    let (s, b) = http(
        port,
        "POST",
        "/api/entities/Plain",
        Some(&token),
        Some(r#"{"title":"same"}"#),
    );
    assert!(s == 200 || s == 201, "insert failed: {b}");
    let plain_id = serde_json::from_str::<Value>(&b).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let note_id = insert_note(port, &token, "note");

    let (s0, r0) = pull(port, &token, 0);
    assert_eq!(s0, 200);
    let cursor = r0["cursor"]["last_seq"].as_u64().unwrap();
    assert!(cursor > 0);

    // Identical update on the plain entity → suppressed: the caller
    // still gets a 200 with the row, but no event is logged.
    let (s1, b1) = http(
        port,
        "PATCH",
        &format!("/api/entities/Plain/{plain_id}"),
        Some(&token),
        Some(r#"{"title":"same"}"#),
    );
    assert_eq!(s1, 200, "noop update must still succeed: {b1}");
    assert_eq!(
        serde_json::from_str::<Value>(&b1).unwrap()["updated"],
        true,
        "noop update must still report success: {b1}"
    );
    let (s2, r2) = pull(port, &token, cursor);
    assert_eq!(s2, 200);
    assert_eq!(
        r2["changes"].as_array().unwrap().len(),
        0,
        "an identical update must not produce a change event: {r2}"
    );
    assert_eq!(
        r2["cursor"]["last_seq"].as_u64().unwrap(),
        cursor,
        "the log must not advance for a noop: {r2}"
    );

    // A REAL update still replicates.
    let (s3, _) = http(
        port,
        "PATCH",
        &format!("/api/entities/Plain/{plain_id}"),
        Some(&token),
        Some(r#"{"title":"different"}"#),
    );
    assert_eq!(s3, 200);
    let (s4, r4) = pull(port, &token, cursor);
    assert_eq!(s4, 200);
    let changes = r4["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "a real update must replicate: {r4}");
    assert_eq!(changes[0]["data"]["title"], "different");

    // CRDT exemption: an identical update on a crdt entity (Note) must
    // STILL produce an event — its Loro state may have advanced even
    // when the materialized JSON did not.
    let cursor2 = r4["cursor"]["last_seq"].as_u64().unwrap();
    let (s5, _) = http(
        port,
        "PATCH",
        &format!("/api/entities/Note/{note_id}"),
        Some(&token),
        Some(r#"{"title":"note"}"#),
    );
    assert_eq!(s5, 200);
    let (s6, r6) = pull(port, &token, cursor2);
    assert_eq!(s6, 200);
    assert_eq!(
        r6["changes"].as_array().unwrap().len(),
        1,
        "a crdt entity must not be noop-suppressed: {r6}"
    );
}

// ---------------------------------------------------------------------------
// `syncOmit` — replicate the row, not the heavy column
// ---------------------------------------------------------------------------

/// A `syncOmit` field must be stripped from every REPLICATION surface
/// (snapshot pull, delta pull, `?sync=1` cursor fetch) while staying on
/// direct reads (entity get, plain cursor). This is the "heavy but not
/// secret" modifier — contrast `serverOnly`, which hides a field from
/// ALL client surfaces.
#[test]
fn sync_omit_strips_replication_but_not_direct_reads() {
    let mut manifest = test_manifest();
    manifest.entities.push(ManifestEntity {
        name: "Doc".into(),
        fields: vec![
            ManifestField {
                name: "title".into(),
                field_type: "string".into(),
                ..Default::default()
            },
            ManifestField {
                name: "renderPlan".into(),
                field_type: "string".into(),
                optional: true,
                sync_omit: true,
                ..Default::default()
            },
        ],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        sync: true,
        ..Default::default()
    });
    manifest.policies.push(ManifestPolicy {
        name: "doc_public".into(),
        entity: Some("Doc".into()),
        allow: "true".into(),
        ..Default::default()
    });
    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    // Anchor so the later pull is a real delta.
    insert_note(port, &token, "anchor");
    let (s0, r0) = pull(port, &token, 0);
    assert_eq!(s0, 200);
    let cursor = r0["cursor"]["last_seq"].as_u64().unwrap();

    let (s, b) = http(
        port,
        "POST",
        "/api/entities/Doc",
        Some(&token),
        Some(r#"{"title":"t1","renderPlan":"HEAVY-BLOB"}"#),
    );
    assert!(s == 200 || s == 201, "insert failed: {b}");
    let id = serde_json::from_str::<Value>(&b).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Delta pull: the insert event ships WITHOUT the heavy column.
    let (s1, r1) = pull(port, &token, cursor);
    assert_eq!(s1, 200);
    let ev = r1["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["entity"] == "Doc")
        .expect("Doc event in delta");
    assert_eq!(ev["data"]["title"], "t1", "{r1}");
    assert!(
        ev["data"].get("renderPlan").is_none(),
        "syncOmit field must not ride the delta feed: {r1}"
    );

    // Snapshot pull (fresh client): same strip.
    let (s2, r2) = pull(port, &token, 0);
    assert_eq!(s2, 200);
    let snap = r2["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["entity"] == "Doc")
        .expect("Doc row in snapshot");
    assert!(
        snap["data"].get("renderPlan").is_none(),
        "syncOmit field must not ride the snapshot: {r2}"
    );

    // Replication cursor fetch (`?sync=1`, the reconcile path): stripped.
    let (s3, b3) = http(
        port,
        "GET",
        "/api/entities/Doc/cursor?limit=100&sync=1",
        Some(&token),
        None,
    );
    assert_eq!(s3, 200);
    let sync_rows: Value = serde_json::from_str(&b3).unwrap();
    assert!(
        sync_rows["data"][0].get("renderPlan").is_none(),
        "syncOmit field must not ride a replication fetch: {b3}"
    );

    // Direct reads KEEP the field — by-id get and the plain cursor.
    let (s4, b4) = http(
        port,
        "GET",
        &format!("/api/entities/Doc/{id}"),
        Some(&token),
        None,
    );
    assert_eq!(s4, 200);
    assert_eq!(
        serde_json::from_str::<Value>(&b4).unwrap()["renderPlan"],
        "HEAVY-BLOB",
        "direct by-id read must keep the field: {b4}"
    );
    let (s5, b5) = http(
        port,
        "GET",
        "/api/entities/Doc/cursor?limit=100",
        Some(&token),
        None,
    );
    assert_eq!(s5, 200);
    assert_eq!(
        serde_json::from_str::<Value>(&b5).unwrap()["data"][0]["renderPlan"],
        "HEAVY-BLOB",
        "a plain cursor read must keep the field: {b5}"
    );
}

/// A `sync_limit` capped entity whose visible TAIL is wider than one
/// snapshot batch must PAGE, not loop. Pre-fix, the overflow
/// continuation was rebuilt from the original resume marker with n=0 —
/// byte-identical to the token the client just sent — so the client
/// re-requested the same page forever and bootstrap never converged
/// (found live on reelbear.app).
#[test]
fn capped_tail_wider_than_a_batch_pages_instead_of_looping() {
    let mut manifest = test_manifest();
    manifest.entities.push(ManifestEntity {
        name: "Feed".into(),
        fields: vec![ManifestField {
            name: "title".into(),
            field_type: "string".into(),
            ..Default::default()
        }],
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: false,
        sync: true,
        sync_limit: Some(1500),
        ..Default::default()
    });
    manifest.policies.push(ManifestPolicy {
        name: "feed_public".into(),
        entity: Some("Feed".into()),
        allow: "true".into(),
        ..Default::default()
    });
    let rt = Arc::new(Runtime::in_memory(manifest).unwrap());
    let port = start_server(rt);
    let token = mint_guest(port);

    // 1200 visible rows — a tail wider than SNAPSHOT_BATCH_LIMIT (1000)
    // but under the 1500 cap, via the real wire path in two batches.
    for batch in 0..2 {
        let ops: Vec<String> = (0..600)
            .map(|i| {
                let n = batch * 600 + i;
                format!(
                    r#"{{"op_id":"f{n}","entity":"Feed","row_id":"f{n}","kind":"insert","data":{{"title":"t{n}"}}}}"#
                )
            })
            .collect();
        let body = format!(r#"{{"changes":[{}]}}"#, ops.join(","));
        let (s, b) = http(port, "POST", "/api/sync/push", Some(&token), Some(&body));
        assert_eq!(s, 200, "batch push failed: {b}");
    }

    // Drain the snapshot, following snapshot_after. Pre-fix this loop
    // never terminates (every page returns the same token), so cap it
    // and fail loudly instead of hanging the suite.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut token_prev: Option<String> = None;
    let mut query = "since=0".to_string();
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(
            pages <= 10,
            "snapshot did not converge in 10 pages — continuation token is looping"
        );
        let (s, body) = http(
            port,
            "GET",
            &format!("/api/sync/pull?{query}"),
            Some(&token),
            None,
        );
        assert_eq!(s, 200, "page {pages}: {body}");
        // Large pages arrive with trailing transfer framing the bare-TCP
        // helper doesn't strip — parse the first JSON value and ignore it.
        let mut de = serde_json::Deserializer::from_str(&body);
        let r: Value = serde::Deserialize::deserialize(&mut de).unwrap();
        for c in r["changes"].as_array().unwrap() {
            if c["entity"] == "Feed" {
                seen.insert(c["row_id"].as_str().unwrap().to_string());
            }
        }
        match r["snapshot_after"].as_str() {
            Some(next) => {
                assert_ne!(
                    Some(next.to_string()),
                    token_prev,
                    "page {pages} returned the SAME continuation token — the pre-fix infinite loop"
                );
                token_prev = Some(next.to_string());
                query = format!("since=0&snapshot_after={next}");
            }
            None => break,
        }
    }
    assert!(pages >= 2, "1200-row tail must span multiple pages");
    assert_eq!(
        seen.len(),
        1200,
        "every capped-tail row must arrive exactly once across pages"
    );
}
