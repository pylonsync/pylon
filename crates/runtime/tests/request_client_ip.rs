//! `ctx.request.clientIp` in an action is the client IP the runtime
//! resolves for rate limits and the audit log.
//!
//! - A plain loopback call sees the socket address.
//! - A configured client-IP header (here `True-Client-IP`) is used.
//! - `CF-Connecting-IP` from a peer that is not a Cloudflare edge is
//!   ignored, so a caller cannot choose its own IP.
//!
//! Needs `bun` on PATH (skipped with a message otherwise).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::AppManifest;
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

fn functions_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "pylon-client-ip-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let fns = dir.join("functions");
    std::fs::create_dir_all(&fns).unwrap();
    let define = repo_root()
        .join("packages/functions/src/define.ts")
        .to_string_lossy()
        .replace('\\', "/");
    std::fs::write(
        fns.join("whereFrom.ts"),
        format!(
            r#"import {{ action }} from "{define}";
export default action({{
  auth: "public",
  handler: async (ctx) => ({{ clientIp: ctx.request?.clientIp ?? "missing" }}),
}});
"#
        ),
    )
    .unwrap();
    fns
}

fn manifest() -> AppManifest {
    serde_json::from_value(serde_json::json!({
        "manifest_version": 1,
        "name": "client-ip",
        "version": "0.1.0",
        "entities": [],
        "routes": [],
        "queries": [],
        "actions": [],
        "policies": []
    }))
    .expect("manifest parses")
}

fn free_port() -> u16 {
    for _ in 0..200 {
        let base = 20_000 + rand::random::<u16>() % 8_000;
        if (0..4).all(|o| TcpListener::bind(("127.0.0.1", base + o)).is_ok()) {
            return base;
        }
    }
    panic!("no free port block");
}

/// POST /api/fn/whereFrom with `extra` headers; returns (status, body).
fn where_from(port: u16, extra: &[(&str, &str)]) -> (u16, serde_json::Value) {
    let extra: String = extra.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
    let req = format!(
        "POST /api/fn/whereFrom HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra}\
         Content-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
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
    (
        status,
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null),
    )
}

#[test]
fn actions_see_the_resolved_client_ip() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let fns = functions_dir();
    // SAFETY: set before the server thread starts; this binary has one test.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_FUNCTIONS_DIR", &fns);
        std::env::set_var(
            "PYLON_FUNCTIONS_RUNTIME",
            repo_root().join("packages/functions/src/runtime.ts"),
        );
        std::env::set_var("PYLON_FN_POOL_SIZE", "1");
        std::env::set_var("PYLON_CLIENT_IP_HEADER", "CF-Connecting-IP,True-Client-IP");
    }
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let port = free_port();
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });

    let deadline = Instant::now() + Duration::from_secs(60);
    let plain = loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            let (status, body) = where_from(port, &[]);
            if status == 200 {
                break body;
            }
        }
        assert!(Instant::now() < deadline, "functions never came up");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(plain["clientIp"], "127.0.0.1", "{plain}");

    let (_, forged) = where_from(port, &[("CF-Connecting-IP", "1.2.3.4")]);
    assert_eq!(forged["clientIp"], "127.0.0.1", "{forged}");

    let (_, configured) = where_from(port, &[("True-Client-IP", "203.0.113.5")]);
    assert_eq!(configured["clientIp"], "203.0.113.5", "{configured}");
}
