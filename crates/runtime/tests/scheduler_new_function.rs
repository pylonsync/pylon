//! `ctx.scheduler.runAfter` reaches a function added after boot. `pylon dev`
//! reloads the functions in place when a file changes; the job handlers
//! used to be registered once at boot, so a job for a new function died
//! with "No handler registered".
//!
//! Needs `bun` on PATH (skipped with a message otherwise).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::*;
use pylon_runtime::Runtime;

const ADMIN: &str = "testadmin_scheduler_new_fn";

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
        "pylon-sched-new-fn-{tag}-{}-{}",
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

fn define_path() -> String {
    repo_root()
        .join("packages/functions/src/define.ts")
        .to_string_lossy()
        .replace('\\', "/")
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "scheduler-new-fn".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Run".into(),
            fields: vec![string_field("by")],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn a_function_added_after_boot_runs_from_the_scheduler() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let define = define_path();
    let fns = tmp_dir("fns").join("functions");
    std::fs::create_dir_all(&fns).unwrap();
    std::fs::write(
        fns.join("scheduleLater.ts"),
        format!(
            r#"import {{ mutation }} from "{define}";
export default mutation({{
  handler: async (ctx) => {{
    await ctx.auth.elevate({{ admin: true, reason: "test: schedule an internal function" }});
    return {{ id: await ctx.scheduler.runAfter(0, "later", {{}}) }};
  }},
}});
"#
        ),
    )
    .unwrap();
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
    std::thread::sleep(Duration::from_secs(2));

    // A new file, then the in-place reload `pylon dev` runs on a change.
    std::fs::write(
        fns.join("later.ts"),
        format!(
            r#"import {{ mutation }} from "{define}";
export default mutation({{
  internal: true,
  handler: async (ctx) => {{
    await ctx.db.insert("Run", {{ by: "later" }});
    return {{ ok: true }};
  }},
}});
"#
        ),
    )
    .unwrap();
    pylon_runtime::frontend::trigger_dev_reload_with(pylon_runtime::frontend::DevReload {
        respawn_runtime: true,
        rebuild_client: false,
    })
    .expect("reload");

    let (status, body) = request(port, "POST", "/api/fn/scheduleLater", Some(ADMIN), "{}");
    assert_eq!(status, 200, "{body}");

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let runs = rt.list("Run").unwrap();
        if runs.iter().any(|r| r["by"] == "later") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the scheduled function never ran"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
