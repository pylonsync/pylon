//! `ctx.files.store` / `ctx.files.delete` from real functions:
//!
//! - an action stores bytes owned by the calling user, who can then fetch
//!   them from `/api/files/<id>`; an anonymous caller cannot;
//! - an action deletes the file, after which it is gone;
//! - a mutation cannot write files (`FILES_WRITE_NOT_ALLOWED`).
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
        "pylon-fn-files-{tag}-{}-{}",
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
    let define = repo_root()
        .join("packages/functions/src/define.ts")
        .to_string_lossy()
        .replace('\\', "/");
    let files = [
        (
            "savePhoto",
            r#"export default action({
  auth: "user",
  handler: async (ctx) =>
    ctx.files.store("race photo bytes", { name: "race.jpg", contentType: "image/jpeg" }),
});"#,
        ),
        (
            "removePhoto",
            r#"export default action({
  auth: "user",
  handler: async (ctx, args) => ctx.files.delete(args.id),
});"#,
        ),
        (
            "storeInMutation",
            r#"export default mutation({
  auth: "public",
  handler: async (ctx) => {
    try {
      await ctx.files.store("x", { name: "x.txt" });
      return { stored: true };
    } catch (e) {
      return { code: e.code ?? String(e) };
    }
  },
});"#,
        ),
    ];
    for (name, body) in files {
        std::fs::write(
            fns.join(format!("{name}.ts")),
            format!("import {{ action, mutation }} from \"{define}\";\n{body}\n"),
        )
        .unwrap();
    }
    fns
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "fn-files".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "User".into(),
            fields: vec![ManifestField {
                name: "email".into(),
                field_type: "string".into(),
                optional: false,
                ..Default::default()
            }],
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

#[test]
fn actions_store_and_delete_files() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let fns = functions_dir();
    let uploads = tmp_dir("uploads");
    // SAFETY: set before the server thread starts; this binary has one test.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_FUNCTIONS_DIR", &fns);
        std::env::set_var(
            "PYLON_FUNCTIONS_RUNTIME",
            repo_root().join("packages/functions/src/runtime.ts"),
        );
        std::env::set_var("PYLON_FN_POOL_SIZE", "1");
        std::env::set_var("PYLON_FILES_DIR", &uploads);
    }
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let port = free_port();
    let server_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(server_rt, port);
    });

    // Wait for the server, then sign in a user.
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
    let user = rt
        .insert("User", &serde_json::json!({ "email": "racer@trux.test" }))
        .unwrap();
    let (status, session) = request(
        port,
        "POST",
        "/api/auth/session",
        None,
        &format!(r#"{{"user_id":"{user}"}}"#),
    );
    assert_eq!(status, 201, "{session}");
    let token = json(&session)["token"].as_str().unwrap().to_string();

    // The functions pool may still be loading: retry the first call.
    let stored = loop {
        let (status, body) = request(port, "POST", "/api/fn/savePhoto", Some(&token), "{}");
        if status == 200 {
            break json(&body);
        }
        assert!(
            Instant::now() < deadline,
            "savePhoto never succeeded: {status} {body}"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    let id = stored["id"].as_str().expect("id").to_string();
    assert_eq!(stored["size"], 16, "{stored}");

    // The owner reads it; an anonymous caller does not.
    let (status, bytes) = request(port, "GET", &format!("/api/files/{id}"), Some(&token), "");
    assert_eq!(status, 200);
    assert_eq!(bytes, "race photo bytes");
    let (status, _) = request(port, "GET", &format!("/api/files/{id}"), None, "");
    assert!(
        status == 401 || status == 404,
        "anonymous read got {status}"
    );

    // Delete, then it is gone.
    let (status, body) = request(
        port,
        "POST",
        "/api/fn/removePhoto",
        Some(&token),
        &format!(r#"{{"id":"{id}"}}"#),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(json(&body)["deleted"], true, "{body}");
    let (status, _) = request(port, "GET", &format!("/api/files/{id}"), Some(&token), "");
    assert_eq!(status, 404);

    // A mutation cannot write files.
    let (status, body) = request(port, "POST", "/api/fn/storeInMutation", None, "{}");
    assert_eq!(status, 200, "{body}");
    assert_eq!(json(&body)["code"], "FILES_WRITE_NOT_ALLOWED", "{body}");
}
