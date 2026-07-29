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
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt2, port);
    });
    for _ in 0..100 {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
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
    let body = match text.find("\r\n\r\n") {
        Some(i) => text[i + 4..].to_string(),
        None => String::new(),
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
    unsafe {
        std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN_TOKEN);
    }

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

    // Budget below the seeded row count forces the scan to paginate.
    unsafe {
        std::env::set_var("PYLON_SNAPSHOT_SCAN_BUDGET", "50");
    }

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
