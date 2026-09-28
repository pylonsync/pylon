//! `field.string().max(n)` (`maxLength` in the manifest) on every write
//! path: the runtime's insert/update, transactions, the entity API, sync
//! push, and `ctx.db` inside a function. CRDT pushes are covered in
//! crdt_parity.rs (both backends).
//!
//! Without the limit, any guest could store values up to the 10 MB body
//! cap, and every client then syncs them.
//!
//! Runs on SQLite, and also on Postgres when `PYLON_TEST_PG_URL` is set
//! (one database, so `--test-threads=1`).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_http::DataStore;
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestPolicy};
use pylon_runtime::Runtime;
use serde_json::{json, Value};

fn field(name: &str, ty: &str, max_length: Option<u32>) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: ty.into(),
        optional: true,
        max_length,
        ..Default::default()
    }
}

/// `Note`: `title` up to 10 characters, `body` (richtext) up to 20,
/// `notes` unlimited. Open policy, so the limit is the only gate.
fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "max-length".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Note".into(),
            fields: vec![
                field("title", "string", Some(10)),
                field("body", "richtext", Some(20)),
                field("notes", "string", None),
            ],
            ..Default::default()
        }],
        policies: vec![ManifestPolicy {
            name: "open".into(),
            entity: Some("Note".into()),
            allow: "true".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// A fresh SQLite runtime, and a fresh Postgres one when a test database
/// is set.
fn runtimes() -> Vec<(&'static str, Runtime)> {
    let mut out = vec![("sqlite", Runtime::in_memory(manifest()).unwrap())];
    if let Ok(url) = std::env::var("PYLON_TEST_PG_URL") {
        let manifest = manifest();
        let mut adapter = pylon_storage::postgres::live::LivePostgresAdapter::connect(&url)
            .expect("connect to test postgres");
        for table in [
            "\"Note\"",
            "_pylon_crdt_snapshots",
            "_pylon_crdt_synthetic",
            "_pylon_crdt_base",
            "_pylon_crdt_links",
        ] {
            let _ = adapter.exec_raw(&format!("DROP TABLE IF EXISTS {table} CASCADE"));
        }
        let plan = adapter.plan_from_live(&manifest).expect("plan");
        adapter.apply_plan(&plan).expect("apply schema");
        out.push((
            "postgres",
            Runtime::open_postgres(&url, manifest).expect("open postgres runtime"),
        ));
    }
    out
}

#[test]
fn runtime_writes_enforce_max_length() {
    for (backend, rt) in runtimes() {
        eprintln!("backend: {backend}");
        check_runtime_writes(&rt);
    }
}

fn check_runtime_writes(rt: &Runtime) {
    // At the limit: fine. Counted in characters, not bytes: ten emoji
    // are 40 bytes.
    let id = rt
        .insert("Note", &json!({ "title": "0123456789", "body": "x" }))
        .unwrap();
    rt.insert("Note", &json!({ "title": "😀".repeat(10) }))
        .unwrap();

    // Over the limit: insert and update both refuse, and nothing lands.
    let err = rt
        .insert("Note", &json!({ "title": "01234567890" }))
        .unwrap_err();
    assert_eq!(err.code, "FIELD_TOO_LONG", "{}", err.message);
    assert!(err.message.contains("Note.title"), "{}", err.message);
    let err = rt
        .update("Note", &id, &json!({ "body": "y".repeat(21) }))
        .unwrap_err();
    assert_eq!(err.code, "FIELD_TOO_LONG", "{}", err.message);
    assert_eq!(rt.get_by_id("Note", &id).unwrap().unwrap()["body"], "x");

    // Fields without a limit, null, and omitted fields pass.
    rt.update(
        "Note",
        &id,
        &json!({ "notes": "n".repeat(10_000), "title": null }),
    )
    .unwrap();

    // The DataStore surface functions use (ctx.db in an action).
    let store: &dyn DataStore = rt;
    let err = store
        .insert("Note", &json!({ "title": "t".repeat(11) }))
        .unwrap_err();
    assert_eq!(err.code, "FIELD_TOO_LONG", "{}", err.message);

    // A transaction (ctx.db inside a mutation, batch writes): the bad op
    // rolls the whole batch back.
    let err = store.transact(&[
        json!({ "op": "insert", "entity": "Note", "data": { "title": "ok" } }),
        json!({ "op": "update", "entity": "Note", "id": id, "data": { "title": "t".repeat(11) } }),
    ]);
    match err {
        Err(e) => assert_eq!(e.code, "FIELD_TOO_LONG", "{}", e.message),
        Ok((committed, results)) => {
            assert!(
                !committed,
                "a batch with a too-long value committed: {results:?}"
            );
            // The batch envelope carries the message, not the code.
            let text = serde_json::to_string(&results).unwrap();
            assert!(
                text.contains("Note.title is 11 characters; the limit is 10"),
                "{text}"
            );
        }
    }
    let titles: Vec<Value> = rt
        .list("Note")
        .unwrap()
        .into_iter()
        .map(|r| r["title"].clone())
        .collect();
    assert!(
        !titles.contains(&json!("ok")),
        "the rolled-back insert landed: {titles:?}"
    );
}

// ---------------------------------------------------------------------------
// HTTP: entity API, sync push, and a function's ctx.db through the real Bun
// runtime.
// ---------------------------------------------------------------------------

fn bun_available() -> bool {
    std::process::Command::new("bun")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    without_verbatim_prefix(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap(),
    )
}

/// `canonicalize` returns a `\\?\` verbatim path on Windows, which bun
/// cannot resolve as a module path. Drop the prefix; other platforms are
/// unchanged.
fn without_verbatim_prefix(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

/// `abs` relative to the current directory: the runtime joins
/// `PYLON_FUNCTIONS_DIR` onto the process cwd.
///
/// Built from path components so it holds on Windows too. There the two
/// paths must share a drive, which is why the test dirs live under
/// `CARGO_TARGET_TMPDIR` (inside the target dir) and not the system temp
/// dir (C: while the checkout may be on D:).
fn relative_to_cwd(abs: &Path) -> String {
    let cwd = without_verbatim_prefix(std::env::current_dir().unwrap().canonicalize().unwrap());
    let abs = without_verbatim_prefix(abs.canonicalize().unwrap());
    let from: Vec<_> = cwd.components().collect();
    let to: Vec<_> = abs.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    assert!(
        common > 0,
        "{} and {} share no root",
        cwd.display(),
        abs.display()
    );
    let mut rel = PathBuf::new();
    for _ in common..from.len() {
        rel.push("..");
    }
    for part in &to[common..] {
        rel.push(part);
    }
    rel.to_string_lossy().into_owned()
}

fn http(port: u16, method: &str, path: &str, token: Option<&str>, body: &str) -> (u16, Value) {
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(t) = token {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|x| x.parse().ok())
        .unwrap_or(0);
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (
        status,
        serde_json::from_str(&body).unwrap_or(Value::String(body)),
    )
}

#[test]
fn http_and_function_writes_enforce_max_length() {
    let with_functions = bun_available();
    if with_functions {
        let fns = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "pylon-max-length-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&fns).unwrap();
        std::fs::write(
            fns.join("addNote.ts"),
            r#"export default {
  type: "mutation",
  auth: "public",
  handler: async (ctx, args) => {
    try {
      return { id: await ctx.db.insert("Note", { title: args.title }) };
    } catch (err) {
      return { error: err.code ?? String(err) };
    }
  },
};
"#,
        )
        .unwrap();
        // SAFETY: the only test in this binary that starts a server; set
        // before the server thread exists.
        unsafe {
            std::env::set_var("PYLON_FUNCTIONS_DIR", relative_to_cwd(&fns));
            std::env::set_var(
                "PYLON_FUNCTIONS_RUNTIME",
                repo_root().join("packages/functions/src/runtime.ts"),
            );
            std::env::set_var("PYLON_FN_POOL_SIZE", "1");
        }
    } else {
        eprintln!("bun is not on PATH: skipping the function part");
    }
    // SAFETY: as above.
    unsafe { std::env::set_var("PYLON_DEV_MODE", "1") };

    for (backend, rt) in runtimes() {
        eprintln!("backend: {backend}");
        check_http_writes(Arc::new(rt), with_functions);
    }
}

fn check_http_writes(rt: Arc<Runtime>, with_functions: bool) {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let server_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(server_rt, port);
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    while TcpStream::connect(format!("127.0.0.1:{port}")).is_err() {
        assert!(Instant::now() < deadline, "server never bound");
        std::thread::sleep(Duration::from_millis(50));
    }

    // A guest session: the caller the limit protects against.
    let (status, guest) = http(port, "POST", "/api/auth/guest", None, "");
    assert_eq!(status, 201, "{guest}");
    let token = guest["token"].as_str().unwrap().to_string();

    // Entity API insert and update.
    let (status, body) = http(
        port,
        "POST",
        "/api/entities/Note",
        Some(&token),
        &json!({ "title": "x".repeat(11) }).to_string(),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "FIELD_TOO_LONG", "{body}");
    let (status, body) = http(
        port,
        "POST",
        "/api/entities/Note",
        Some(&token),
        &json!({ "title": "fits" }).to_string(),
    );
    assert!(status == 200 || status == 201, "{body}");
    let id = body["id"].as_str().unwrap().to_string();
    let (status, body) = http(
        port,
        "PATCH",
        &format!("/api/entities/Note/{id}"),
        Some(&token),
        &json!({ "body": "b".repeat(21) }).to_string(),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "FIELD_TOO_LONG", "{body}");

    // Sync push: the op fails, the other op in the batch still applies.
    let push = json!({ "changes": [
        { "op_id": "long", "entity": "Note", "row_id": id, "kind": "update",
          "data": { "title": "x".repeat(5_000) } },
        { "op_id": "short", "entity": "Note", "row_id": id, "kind": "update",
          "data": { "title": "short" } },
    ]});
    let (status, body) = http(
        port,
        "POST",
        "/api/sync/push",
        Some(&token),
        &push.to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let results = body["results"].as_array().expect("results");
    let long = results.iter().find(|r| r["op_id"] == "long").unwrap();
    assert_eq!(long["status"], "error", "{body}");
    assert_eq!(long["error"]["code"], "FIELD_TOO_LONG", "{body}");
    let short = results.iter().find(|r| r["op_id"] == "short").unwrap();
    assert_eq!(short["status"], "applied", "{body}");
    assert_eq!(
        rt.get_by_id("Note", &id).unwrap().unwrap()["title"],
        "short"
    );

    if with_functions {
        // ctx.db.insert inside a mutation.
        let deadline = Instant::now() + Duration::from_secs(60);
        let body = loop {
            let (status, body) = http(
                port,
                "POST",
                "/api/fn/addNote",
                Some(&token),
                &json!({ "title": "x".repeat(11) }).to_string(),
            );
            if status == 200 {
                break body;
            }
            assert!(Instant::now() < deadline, "functions never came up: {body}");
            std::thread::sleep(Duration::from_millis(200));
        };
        assert_eq!(body["error"], "FIELD_TOO_LONG", "{body}");
        let (status, body) = http(
            port,
            "POST",
            "/api/fn/addNote",
            Some(&token),
            &json!({ "title": "fine" }).to_string(),
        );
        assert_eq!(status, 200, "{body}");
        assert!(body["id"].is_string(), "{body}");
    }
}
