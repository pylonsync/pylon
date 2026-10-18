//! `afterEdit` from a real function: every write to a `Note` row within one
//! window schedules the internal mutation `indexNote` once; it runs at the
//! window's end and records a `Run` row. Writes go through the HTTP entity
//! API, the path that notifies.
//!
//! Needs `bun` on PATH (skipped with a message otherwise).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::*;
use pylon_runtime::Runtime;

const ADMIN: &str = "testadmin_after_edit_hook";
const WINDOW: u64 = 2;

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
        "pylon-after-edit-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn string_field(name: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional: false,
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

fn functions_dir() -> PathBuf {
    let fns = tmp_dir("fns").join("functions");
    std::fs::create_dir_all(&fns).unwrap();
    let define = repo_root()
        .join("packages/functions/src/define.ts")
        .to_string_lossy()
        .replace('\\', "/");
    std::fs::write(
        fns.join("indexNote.ts"),
        format!(
            r#"import {{ mutation }} from "{define}";
export default mutation({{
  internal: true,
  handler: async (ctx, args) => {{
    await ctx.db.insert("Run", {{ noteId: args.id }});
    return {{ ok: true }};
  }},
}});
"#
        ),
    )
    .unwrap();
    fns
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "after-edit".into(),
        version: "0.1.0".into(),
        entities: vec![
            ManifestEntity {
                name: "Note".into(),
                fields: vec![string_field("title")],
                crdt: true,
                sync: true,
                after_edit: Some(ManifestAfterEdit {
                    function: "indexNote".into(),
                    delay_secs: WINDOW,
                }),
                ..Default::default()
            },
            ManifestEntity {
                name: "Run".into(),
                fields: vec![string_field("noteId")],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn edits_in_a_window_run_the_hook_once() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let fns = functions_dir();
    // SAFETY: set before the server thread starts; this binary has one test.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN);
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
    // Let the functions pool load, so the first run is not a retry.
    std::thread::sleep(Duration::from_secs(2));

    let first_write = now_secs();
    let (status, body) = request(
        port,
        "POST",
        "/api/entities/Note",
        Some(ADMIN),
        r#"{"title":"t0"}"#,
    );
    assert!(status == 200 || status == 201, "{status} {body}");
    let note = json(&body)["id"].as_str().expect("note id").to_string();
    for i in 1..=5 {
        let (status, body) = request(
            port,
            "PATCH",
            &format!("/api/entities/Note/{note}"),
            Some(ADMIN),
            &format!(r#"{{"title":"t{i}"}}"#),
        );
        assert_eq!(status, 200, "{body}");
    }
    let last_write = now_secs();
    let windows = (last_write / WINDOW - first_write / WINDOW + 1) as usize;

    let runs = |rt: &Runtime| -> usize {
        rt.list("Run")
            .unwrap()
            .iter()
            .filter(|r| r["noteId"] == note.as_str())
            .count()
    };
    let deadline = Instant::now() + Duration::from_secs(WINDOW * 2 + 20);
    while runs(&rt) < windows {
        assert!(
            Instant::now() < deadline,
            "the hook ran {} of {windows} times",
            runs(&rt)
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    // And no more after that: one run per window, not one per write.
    std::thread::sleep(Duration::from_secs(WINDOW + 1));
    assert_eq!(runs(&rt), windows, "six writes in {windows} window(s)");
}
