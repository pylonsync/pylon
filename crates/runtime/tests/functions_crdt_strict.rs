//! `ctx.crdt` under strict function policies (`PYLON_STRICT_FN_POLICIES=1`),
//! the `ctx.db` rule:
//!
//! - the owner edits their note;
//! - another user's edit is refused (`POLICY_DENIED`) and their open finds
//!   nothing (`NOT_FOUND`, the same answer as a missing row);
//! - `ctx.crdt.unsafe` (`openLiveDoc(..., { unsafe: true })`) runs with
//!   server trust.
//!
//! Needs `bun` on PATH (skipped with a message otherwise).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::*;
use pylon_runtime::Runtime;

fn bun_available() -> bool {
    std::process::Command::new("bun")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "pylon-fn-crdt-strict-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn functions_dir() -> PathBuf {
    let fns = tmp_dir("fns").join("functions");
    std::fs::create_dir_all(&fns).unwrap();
    let root = repo_root();
    let define = root
        .join("packages/functions/src/define.ts")
        .to_string_lossy()
        .replace('\\', "/");
    let server = root
        .join("packages/loro/src/server.ts")
        .to_string_lossy()
        .replace('\\', "/");
    let files = [(
        "setTitle",
        r#"export default action({
  auth: "user",
  handler: async (ctx, args) => {
    try {
      const live = await openLiveDoc(ctx, "Note", args.id, { unsafe: args.unsafe === true });
      await live.edit((doc) => doc.getMap("row").set("title", args.title));
      return { ok: true };
    } catch (e) {
      return { code: e.code ?? String(e) };
    }
  },
});"#,
    )];
    for (name, body) in files {
        std::fs::write(
            fns.join(format!("{name}.ts")),
            format!(
                "import {{ action, mutation }} from \"{define}\";\nimport {{ openLiveDoc }} from \"{server}\";\n{body}\n"
            ),
        )
        .unwrap();
    }
    fns
}

fn string_field(name: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional: false,
        ..Default::default()
    }
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "fn-crdt".into(),
        version: "0.1.0".into(),
        entities: vec![
            ManifestEntity {
                name: "User".into(),
                fields: vec![string_field("email")],
                ..Default::default()
            },
            ManifestEntity {
                name: "Note".into(),
                fields: vec![string_field("title"), string_field("ownerId")],
                crdt: true,
                sync: true,
                ..Default::default()
            },
        ],
        policies: vec![ManifestPolicy {
            name: "ownerOnly".into(),
            entity: Some("Note".into()),
            allow_read: Some("auth.userId == data.ownerId".into()),
            allow_update: Some("auth.userId == data.ownerId".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
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

fn request(port: u16, method: &str, path: &str, token: Option<&str>, body: &str) -> (u16, String) {
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap_or(serde_json::Value::Null)
}

fn session(port: u16, user: &str) -> String {
    let (status, body) = request(
        port,
        "POST",
        "/api/auth/session",
        None,
        &format!(r#"{{"user_id":"{user}"}}"#),
    );
    assert_eq!(status, 201, "{body}");
    json(&body)["token"].as_str().unwrap().to_string()
}

#[test]
fn strict_policies_gate_live_document_edits() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let fns = functions_dir();
    // SAFETY: set before the server thread starts; this binary has one test.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_STRICT_FN_POLICIES", "1");
        std::env::set_var("PYLON_FUNCTIONS_DIR", &fns);
        std::env::set_var(
            "PYLON_FUNCTIONS_RUNTIME",
            repo_root().join("packages/functions/src/runtime.ts"),
        );
        std::env::set_var("PYLON_FN_POOL_SIZE", "1");
    }
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let port = free_port();
    let server_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(server_rt, port);
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok()
            && request(port, "GET", "/health", None, "").0 == 200
        {
            break;
        }
        assert!(Instant::now() < deadline, "server never came up");
        std::thread::sleep(Duration::from_millis(100));
    }
    let owner = rt
        .insert("User", &serde_json::json!({ "email": "owner@draft.test" }))
        .unwrap();
    let other = rt
        .insert("User", &serde_json::json!({ "email": "other@draft.test" }))
        .unwrap();
    let owner_token = session(port, &owner);
    let other_token = session(port, &other);
    let note = rt
        .insert(
            "Note",
            &serde_json::json!({ "title": "draft", "ownerId": owner }),
        )
        .unwrap();
    let call = |token: &str, title: &str, unsafe_op: bool| {
        let args = format!(r#"{{"id":"{note}","title":"{title}","unsafe":{unsafe_op}}}"#);
        request(port, "POST", "/api/fn/setTitle", Some(token), &args)
    };

    // The functions pool may still be loading: retry the first call.
    loop {
        let (status, body) = call(&owner_token, "by the owner", false);
        if status == 200 {
            assert_eq!(json(&body)["ok"], true, "{body}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "setTitle never succeeded: {status} {body}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(
        rt.get_by_id("Note", &note).unwrap().unwrap()["title"],
        "by the owner"
    );

    // Another user may not read the note, so the open finds nothing.
    let (status, body) = call(&other_token, "by someone else", false);
    assert_eq!(status, 200, "{body}");
    assert_eq!(json(&body)["code"], "NOT_FOUND", "{body}");
    assert_eq!(
        rt.get_by_id("Note", &note).unwrap().unwrap()["title"],
        "by the owner"
    );

    // Server trust, as ctx.db.unsafe.
    let (status, body) = call(&other_token, "with server trust", true);
    assert_eq!(status, 200, "{body}");
    assert_eq!(json(&body)["ok"], true, "{body}");
    assert_eq!(
        rt.get_by_id("Note", &note).unwrap().unwrap()["title"],
        "with server trust"
    );
}
