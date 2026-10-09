//! Update policies hold for the row before AND after the write.
//!
//! With the tenant rule `allowUpdate: "auth.tenantId == data.orgId"`, a
//! member of org A must not be able to move a row into org B through any
//! client write path: REST PATCH, `/api/sync/push`, `/api/link`,
//! `/api/unlink`, or a CRDT push to `/api/crdt/<entity>/<id>`. Each path is
//! exercised against a real server and a real store, and each test checks
//! the stored row afterwards, not only the status code.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::*;
use pylon_runtime::Runtime;
use serde_json::{json, Value};

const ADMIN_TOKEN: &str = "update_policy_scope_admin";
/// Org ids. Client-supplied row ids must be 40-char lowercase hex.
const ORG_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ORG_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn field(name: &str, readonly: bool) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional: true,
        unique: false,
        crdt: None,
        server_only: false,
        readonly,
        default: None,
        enum_values: None,
        encrypted: false,
        sync_omit: false,
        max_length: None,
    }
}

fn manifest() -> AppManifest {
    let tenant_rule = "auth.tenantId == data.orgId";
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "update-policy-scope".into(),
        version: "0.1.0".into(),
        entities: vec![
            ManifestEntity {
                name: "Org".into(),
                fields: vec![field("name", false)],
                ..Default::default()
            },
            ManifestEntity {
                name: "OrgMember".into(),
                fields: vec![
                    field("orgId", false),
                    field("userId", false),
                    field("role", false),
                ],
                ..Default::default()
            },
            // Tenant column is writable: only the update policy guards it.
            ManifestEntity {
                name: "Project".into(),
                fields: vec![field("orgId", false), field("name", false)],
                relations: vec![ManifestRelation {
                    name: "org".into(),
                    target: "Org".into(),
                    field: "orgId".into(),
                    many: false,
                }],
                crdt: true,
                ..Default::default()
            },
            // Same shape with a readonly tenant column.
            ManifestEntity {
                name: "Doc".into(),
                fields: vec![field("orgId", true), field("name", false)],
                relations: vec![ManifestRelation {
                    name: "org".into(),
                    target: "Org".into(),
                    field: "orgId".into(),
                    many: false,
                }],
                crdt: true,
                ..Default::default()
            },
            // An encrypted readonly column on a CRDT entity.
            ManifestEntity {
                name: "Note".into(),
                fields: vec![
                    field("orgId", false),
                    field("name", false),
                    ManifestField {
                        encrypted: true,
                        server_only: true,
                        ..field("secret", true)
                    },
                ],
                crdt: true,
                ..Default::default()
            },
        ],
        policies: ["Project", "Doc", "Note"]
            .iter()
            .map(|e| ManifestPolicy {
                name: format!("{e}_tenant"),
                entity: Some((*e).into()),
                allow_read: Some(tenant_rule.into()),
                allow_insert: Some(tenant_rule.into()),
                allow_update: Some(tenant_rule.into()),
                allow_delete: Some(tenant_rule.into()),
                ..Default::default()
            })
            .chain(["Org", "OrgMember"].iter().map(|e| ManifestPolicy {
                name: format!("{e}_admin"),
                entity: Some((*e).into()),
                allow: "auth.isAdmin".into(),
                ..Default::default()
            }))
            .collect(),
        ..Default::default()
    }
}

fn request(port: u16, method: &str, path: &str, body: &str, token: Option<&str>) -> (u16, Value) {
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{auth}Connection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.write_all(req.as_bytes()).expect("write");
    let mut resp = String::new();
    stream.read_to_string(&mut resp).ok();
    let status = resp
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = resp.split("\r\n\r\n").nth(1).unwrap_or("");
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

struct App {
    port: u16,
    runtime: Arc<Runtime>,
    /// Session for alice, a member of org-a with org-a selected.
    alice: String,
}

fn free_port() -> u16 {
    // The server binds port..port+3 (HTTP, WS, SSE, shard WS).
    for _ in 0..200 {
        let base = 20_000 + rand::random::<u16>() % 8_000;
        let ok = (0..4).all(|o| pylon_runtime::listen::port_is_free(base + o));
        if ok {
            return base;
        }
    }
    panic!("no free port block");
}

fn start() -> App {
    start_manifest(manifest())
}

fn start_manifest(schema: AppManifest) -> App {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        // SAFETY: once per binary, before any server thread starts.
        unsafe {
            std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN_TOKEN);
            std::env::set_var("PYLON_DEV_MODE", "1");
            std::env::set_var(
                "PYLON_ENCRYPTION_KEY",
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            );
        }
    });
    let port = free_port();
    let runtime = Arc::new(Runtime::in_memory(schema).unwrap());
    let rt = Arc::clone(&runtime);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });
    // A bound listener accepts connections before the server answers
    // requests, so wait for a real response.
    let ready = (0..600).any(|_| {
        if TcpStream::connect(("127.0.0.1", port)).is_ok()
            && request(port, "GET", "/health", "", None).0 == 200
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
        false
    });
    assert!(ready, "server on port {port} never answered /health");
    for org in [ORG_A, ORG_B] {
        runtime
            .insert("Org", &json!({"id": org, "name": org}))
            .unwrap();
    }
    runtime
        .insert(
            "OrgMember",
            &json!({"orgId": ORG_A, "userId": "alice", "role": "member"}),
        )
        .unwrap();
    let (s, body) = request(
        port,
        "POST",
        "/api/auth/session",
        r#"{"user_id":"alice"}"#,
        None,
    );
    assert!(s == 200 || s == 201, "session: {s} {body}");
    let alice = body["token"].as_str().unwrap().to_string();
    let (s, body) = request(
        port,
        "POST",
        "/api/auth/select-org",
        &json!({ "orgId": ORG_A }).to_string(),
        Some(&alice),
    );
    assert_eq!(s, 200, "select-org: {body}");
    App {
        port,
        runtime,
        alice,
    }
}

impl App {
    fn call(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        request(
            self.port,
            method,
            path,
            &body.to_string(),
            Some(&self.alice),
        )
    }

    fn insert(&self, entity: &str, org: &str) -> String {
        let (s, body) = self.call(
            "POST",
            &format!("/api/entities/{entity}"),
            json!({"orgId": org, "name": "Roadmap"}),
        );
        assert_eq!(s, 201, "insert {entity}: {body}");
        body["id"].as_str().unwrap().to_string()
    }

    fn stored(&self, entity: &str, id: &str) -> Value {
        use pylon_http::DataStore;
        DataStore::get_by_id(self.runtime.as_ref(), entity, id)
            .unwrap()
            .expect("row")
    }

    fn crdt_push(&self, entity: &str, id: &str, set: &[(&str, &str)]) -> (u16, Value) {
        use pylon_crdt::{encode_snapshot, loro::LoroDoc, root_map};
        use pylon_http::DataStore;
        let snap = DataStore::crdt_snapshot(self.runtime.as_ref(), entity, id)
            .unwrap()
            .unwrap();
        let peer = LoroDoc::new();
        pylon_crdt::apply_update(&peer, &snap).unwrap();
        for (k, v) in set {
            root_map(&peer).insert(k, *v).unwrap();
        }
        peer.commit();
        let hex: String = encode_snapshot(&peer)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        self.call(
            "POST",
            &format!("/api/crdt/{entity}/{id}"),
            json!({ "update": hex }),
        )
    }
}

#[test]
fn rest_patch_cannot_move_a_row_to_another_tenant() {
    let app = start();
    let id = app.insert("Project", ORG_A);

    let (s, body) = app.call(
        "PATCH",
        &format!("/api/entities/Project/{id}"),
        json!({"orgId": ORG_B}),
    );
    assert_eq!(s, 403, "cross-tenant PATCH must be denied: {body}");
    assert_eq!(app.stored("Project", &id)["orgId"], ORG_A);

    let (s, body) = app.call(
        "PATCH",
        &format!("/api/entities/Project/{id}"),
        json!({"name": "Q3"}),
    );
    assert_eq!(s, 200, "same-tenant PATCH must pass: {body}");
    assert_eq!(app.stored("Project", &id)["name"], "Q3");
}

#[test]
fn sync_push_cannot_move_a_row_to_another_tenant() {
    let app = start();
    let id = app.insert("Project", ORG_A);

    let (s, body) = app.call(
        "POST",
        "/api/sync/push",
        json!({"changes": [
            {"entity": "Project", "row_id": id, "kind": "update", "data": {"orgId": ORG_B}},
            {"entity": "Project", "row_id": id, "kind": "update", "data": {"name": "Q3"}}
        ]}),
    );
    assert_eq!(s, 200, "push: {body}");
    let results = body["results"].as_array().expect("results");
    assert_eq!(results[0]["status"], "error", "move must fail: {body}");
    assert_eq!(results[0]["error"]["code"], "POLICY_DENIED");
    assert_eq!(
        results[1]["status"], "applied",
        "plain edit must apply: {body}"
    );
    let row = app.stored("Project", &id);
    assert_eq!(row["orgId"], ORG_A);
    assert_eq!(row["name"], "Q3");
}

#[test]
fn link_and_unlink_cannot_move_a_row_out_of_the_tenant() {
    let app = start();
    let id = app.insert("Project", ORG_A);

    let (s, body) = app.call(
        "POST",
        "/api/link",
        json!({"entity": "Project", "id": id, "relation": "org", "target_id": ORG_B}),
    );
    assert_eq!(s, 403, "link into another tenant: {body}");
    let (s, body) = app.call(
        "POST",
        "/api/unlink",
        json!({"entity": "Project", "id": id, "relation": "org"}),
    );
    assert_eq!(s, 403, "unlink out of the tenant: {body}");
    assert_eq!(app.stored("Project", &id)["orgId"], ORG_A);

    // Relinking to the caller's own org is not a move.
    let (s, body) = app.call(
        "POST",
        "/api/link",
        json!({"entity": "Project", "id": id, "relation": "org", "target_id": ORG_A}),
    );
    assert_eq!(s, 200, "same-tenant link: {body}");
}

#[test]
fn link_cannot_write_a_readonly_column() {
    let app = start();
    let id = app.insert("Doc", ORG_A);
    let (s, body) = app.call(
        "POST",
        "/api/link",
        json!({"entity": "Doc", "id": id, "relation": "org", "target_id": ORG_A}),
    );
    assert_eq!(s, 400, "readonly FK via link: {body}");
    assert_eq!(body["error"]["code"], "READONLY_FIELD");
}

#[test]
fn crdt_push_cannot_move_a_row_to_another_tenant() {
    let app = start();
    let id = app.insert("Project", ORG_A);

    let (s, body) = app.crdt_push("Project", &id, &[("orgId", ORG_B)]);
    assert_eq!(s, 403, "cross-tenant CRDT push: {body}");
    assert_eq!(app.stored("Project", &id)["orgId"], ORG_A);

    // The rejected merge must not linger in the cached doc: a later edit
    // from a fresh snapshot keeps the original tenant.
    let (s, body) = app.crdt_push("Project", &id, &[("name", "Q3")]);
    assert_eq!(s, 200, "same-tenant CRDT push: {body}");
    let row = app.stored("Project", &id);
    assert_eq!(row["orgId"], ORG_A);
    assert_eq!(row["name"], "Q3");
}

#[test]
fn crdt_push_cannot_change_a_readonly_column() {
    let app = start();
    let id = app.insert("Doc", ORG_A);
    let (s, body) = app.crdt_push("Doc", &id, &[("orgId", ORG_B)]);
    assert_eq!(s, 400, "readonly column via CRDT: {body}");
    assert_eq!(app.stored("Doc", &id)["orgId"], ORG_A);
    // Untouched readonly columns don't block ordinary edits.
    let (s, body) = app.crdt_push("Doc", &id, &[("name", "Q3")]);
    assert_eq!(s, 200, "CRDT edit of a writable column: {body}");
}

#[test]
fn foreign_rows_cannot_be_updated_or_deleted() {
    let app = start();
    let foreign = app
        .runtime
        .insert("Project", &json!({"orgId": ORG_B, "name": "theirs"}))
        .unwrap();

    let (s, _) = app.call(
        "PATCH",
        &format!("/api/entities/Project/{foreign}"),
        json!({"orgId": ORG_A}),
    );
    assert_eq!(s, 403, "pulling a foreign row into your tenant");
    let (s, _) = app.call(
        "DELETE",
        &format!("/api/entities/Project/{foreign}"),
        json!({}),
    );
    assert_eq!(s, 403, "deleting a foreign row");
    assert_eq!(app.stored("Project", &foreign)["orgId"], ORG_B);

    let own = app.insert("Project", ORG_A);
    let (s, body) = app.call("DELETE", &format!("/api/entities/Project/{own}"), json!({}));
    assert_eq!(s, 200, "deleting your own row: {body}");
}

/// An insert that reuses another tenant's row id must fail without touching
/// that row's CRDT doc. Before the fix, the SQLite path seeded the victim's
/// cached doc with the insert payload, then rolled back only the SQL.
#[test]
fn insert_with_a_taken_id_leaves_the_other_rows_doc_alone() {
    let app = start();
    let foreign = app
        .runtime
        .insert("Project", &json!({"orgId": ORG_B, "name": "theirs"}))
        .unwrap();

    let (s, body) = app.call(
        "POST",
        "/api/entities/Project",
        json!({"id": foreign, "orgId": ORG_A, "name": "poison"}),
    );
    assert!(s >= 400, "insert over a foreign id: {s} {body}");

    use pylon_http::DataStore;
    let snap = DataStore::crdt_snapshot(app.runtime.as_ref(), "Project", &foreign)
        .unwrap()
        .unwrap();
    let doc = pylon_crdt::loro::LoroDoc::new();
    pylon_crdt::apply_update(&doc, &snap).unwrap();
    let state = format!("{:?}", pylon_crdt::root_map(&doc).get_deep_value());
    assert!(
        !state.contains("poison"),
        "doc was seeded by the insert: {state}"
    );
    assert!(state.contains(ORG_B), "{state}");
}

/// Binary history can contain private ciphertext. Reject CRDT pushes and
/// keep JSON updates available beside an encrypted readonly column.
#[test]
fn encrypted_columns_require_json_updates() {
    let app = start();
    let (s, body) = app.call(
        "POST",
        "/api/entities/Note",
        json!({"orgId": ORG_A, "name": "n", "secret": "s3cret"}),
    );
    assert_eq!(s, 201, "{body}");
    let id = body["id"].as_str().unwrap().to_string();
    assert_eq!(app.stored("Note", &id)["secret"], "s3cret");

    let (s, body) = app.crdt_push("Note", &id, &[("name", "renamed")]);
    assert_eq!(s, 403, "private binary history must be blocked: {body}");
    assert_eq!(app.stored("Note", &id)["name"], "n");
    let (s, body) = app.call(
        "PATCH",
        &format!("/api/entities/Note/{id}"),
        json!({"name": "renamed"}),
    );
    assert_eq!(
        s, 200,
        "JSON update beside an encrypted readonly column: {body}"
    );
    assert!(body.get("secret").is_none(), "private plaintext: {body}");
    let row = app.stored("Note", &id);
    assert_eq!(row["name"], "renamed");
    assert_eq!(row["secret"], "s3cret");
}

#[test]
fn client_queries_cannot_probe_private_fields() {
    let mut schema = manifest();
    schema
        .policies
        .iter_mut()
        .find(|policy| policy.entity.as_deref() == Some("Note"))
        .unwrap()
        .allow_read = Some("true".into());
    let app = start_manifest(schema);
    let (status, body) = app.call(
        "POST",
        "/api/entities/Note",
        json!({"orgId": ORG_A, "name": "public", "secret": "hidden"}),
    );
    assert_eq!(status, 201, "{body}");
    for spec in [
        json!({"min":["secret"]}),
        json!({"max":["secret"]}),
        json!({"count":"secret"}),
        json!({"countDistinct":["secret"]}),
        json!({"count":"*", "groupBy":["secret"]}),
        json!({"count":"*", "groupBy":[{"field":"secret"}]}),
        json!({"count":"*", "where":{"secret":"hidden"}}),
    ] {
        let (status, body) = app.call("POST", "/api/aggregate/Note", spec.clone());
        assert_eq!(status, 403, "{spec}: {body}");
        assert!(body.to_string().contains("FIELD_NOT_PUBLIC"));
    }
    for filter in [
        json!({"secret":"hidden"}),
        json!({"$order":{"secret":"asc"}}),
    ] {
        let (status, body) = app.call("POST", "/api/query/Note", filter);
        assert_eq!(status, 403, "{body}");
    }
    for path in [
        "/api/lookup/Note/secret/hidden",
        "/api/entities/Note?sort=secret",
        "/api/entities/Note?filter[secret]=hidden",
    ] {
        let (status, body) = app.call("GET", path, json!(null));
        assert_eq!(status, 403, "{path}: {body}");
    }
    let (status, body) = app.call(
        "POST",
        "/api/aggregate/Note",
        json!({"count":"*", "groupBy":["name"]}),
    );
    assert_eq!(status, 200, "{body}");
    let (status, body) = app.call("POST", "/api/query/Note", json!({"name":"public"}));
    assert_eq!(status, 200, "{body}");
    assert!(!body.to_string().contains("secret"));
}

#[test]
fn bulk_reads_reject_row_rules_that_pass_without_a_row() {
    let mut schema = manifest();
    schema
        .policies
        .iter_mut()
        .find(|policy| policy.entity.as_deref() == Some("Doc"))
        .unwrap()
        .allow_read = Some("data.name != 'hidden'".into());
    let app = start_manifest(schema);
    app.runtime
        .insert("Doc", &json!({"orgId": ORG_A, "name":"hidden"}))
        .unwrap();
    app.runtime
        .insert("Doc", &json!({"orgId": ORG_A, "name":"visible"}))
        .unwrap();
    for (path, query) in [
        ("/api/aggregate/Doc", json!({"count":"*"})),
        ("/api/query", json!({"Doc":{}})),
        ("/api/search/Doc", json!({"q":""})),
        (
            "/api/vector-search/Doc",
            json!({"field":"embedding", "vector":[1.0]}),
        ),
    ] {
        let (status, body) = app.call("POST", path, query);
        assert_eq!(status, 403, "{path}: {body}");
    }
    let (status, body) = app.call("POST", "/api/query/Doc", json!({}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["name"], "visible");
}

#[test]
fn list_totals_use_sql_counts_only_for_row_independent_policies() {
    static COUNTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn trace(event: rusqlite::trace::TraceEvent<'_>) {
        let rusqlite::trace::TraceEvent::Stmt(_, sql) = event else {
            return;
        };
        if sql.starts_with("SELECT COUNT(*) FROM (") && sql.contains("pylon_count") {
            COUNTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    for (rule, expected_total, expected_queries) in
        [("true", 2, 1), ("auth.tenantId == existing.orgId", 1, 0)]
    {
        let mut schema = manifest();
        schema
            .policies
            .iter_mut()
            .find(|policy| policy.entity.as_deref() == Some("Doc"))
            .unwrap()
            .allow_read = Some(rule.into());
        let app = start_manifest(schema);
        app.runtime
            .insert("Doc", &json!({"orgId":ORG_A,"name":"ours"}))
            .unwrap();
        app.runtime
            .insert("Doc", &json!({"orgId":ORG_B,"name":"theirs"}))
            .unwrap();
        COUNTS.store(0, std::sync::atomic::Ordering::SeqCst);
        app.runtime.lock_conn_pub().unwrap().trace_v2(
            rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
            Some(trace),
        );
        let (status, body) = app.call("GET", "/api/entities/Doc?page=1&per_page=1", json!(null));
        app.runtime
            .lock_conn_pub()
            .unwrap()
            .trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["total"], expected_total, "{rule}: {body}");
        assert_eq!(
            COUNTS.load(std::sync::atomic::Ordering::SeqCst),
            expected_queries
        );
    }
}

#[test]
fn bulk_import_and_export_require_admin_without_an_active_tenant() {
    let mut schema = manifest();
    schema.auth.user.admin_field = Some("isAdmin".into());
    schema.entities.push(ManifestEntity {
        name: "User".into(),
        fields: vec![ManifestField {
            field_type: "bool".into(),
            server_only: true,
            ..field("isAdmin", false)
        }],
        ..Default::default()
    });
    let app = start_manifest(schema);
    // The shared session fixture uses a legacy, non-hex user ID.
    app.runtime
        .lock_conn_pub()
        .unwrap()
        .execute(
            "INSERT INTO User(id, isAdmin) VALUES (?1, ?2)",
            rusqlite::params!["alice", true],
        )
        .unwrap();
    // This policy requires admin status. Confirm session elevation before
    // testing the tenant restriction, so a plain-user denial cannot pass.
    assert_eq!(app.call("GET", "/api/entities/Org", Value::Null).0, 200);
    let foreign_id = app
        .runtime
        .insert("Doc", &json!({"orgId":ORG_B,"name":"foreign"}))
        .unwrap();
    for path in ["/api/export", "/api/export/Doc"] {
        let (status, body) = app.call("GET", path, Value::Null);
        assert_eq!(status, 403, "{path}: {body}");
        assert!(!body.to_string().contains("foreign"));
        let (status, body) = request(app.port, "GET", path, "", Some(ADMIN_TOKEN));
        assert_eq!(status, 200, "{path}: {body}");
        assert_eq!(body["entities"]["Doc"][0]["id"], foreign_id);
    }
    let bundle = json!({"entities":{"Doc":[{"orgId":ORG_B,"name":"imported"}]}});
    let (status, body) = app.call("POST", "/api/import", bundle.clone());
    assert_eq!(status, 403, "{body}");
    assert_eq!(app.runtime.list("Doc").unwrap().len(), 1);
    let (status, body) = request(
        app.port,
        "POST",
        "/api/import",
        &bundle.to_string(),
        Some(ADMIN_TOKEN),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["imported"], 1);
    assert_eq!(app.runtime.list("Doc").unwrap().len(), 2);
}
