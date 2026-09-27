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
        ],
        policies: ["Project", "Doc"]
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
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
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
        let ok = (0..4).all(|o| std::net::TcpListener::bind(("127.0.0.1", base + o)).is_ok());
        if ok {
            return base;
        }
    }
    panic!("no free port block");
}

fn start() -> App {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        // SAFETY: once per binary, before any server thread starts.
        unsafe {
            std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN_TOKEN);
            std::env::set_var("PYLON_DEV_MODE", "1");
        }
    });
    let port = free_port();
    let runtime = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let rt = Arc::clone(&runtime);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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
