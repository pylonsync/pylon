//! A shard kind's `onStop` function runs as a job when one of its shards
//! ends: here an action creates and stops a shard, and `shardEnded`
//! records `{ shardId, kind, reason }`.
//!
//! Needs `bun` on PATH and the wasm32-unknown-unknown target (skipped with
//! a message when bun is missing).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::*;
use pylon_runtime::Runtime;

fn bun_available() -> bool {
    Command::new("bun")
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

/// The `zone` guest module, built once.
fn zone_wasm() -> PathBuf {
    let root = repo_root();
    // A target dir of its own, so this build never waits on the lock the
    // outer `cargo test` holds.
    let target_dir = root.join("target/shard-guest-tests");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args([
            "build",
            "-p",
            "pylon-shard-guest",
            "--examples",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--target-dir",
        ])
        .arg(&target_dir)
        .status()
        .expect("run cargo");
    assert!(status.success(), "building the guest examples failed");
    target_dir.join("wasm32-unknown-unknown/release/examples/zone.wasm")
}

fn functions_dir() -> PathBuf {
    let fns = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "pylon-shard-on-stop-{}-{}/functions",
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&fns).unwrap();
    let define = repo_root()
        .join("packages/functions/src/define.ts")
        .to_string_lossy()
        .replace('\\', "/");
    let files = [
        (
            "shardEnded",
            r#"export default mutation({
  internal: true,
  handler: async (ctx, args) => {
    await ctx.db.insert("Ended", { shardId: args.shardId, kind: args.kind, reason: args.reason });
  },
});"#,
        ),
        (
            "startAndStop",
            r#"export default action({
  auth: "public",
  handler: async (ctx) => {
    await ctx.shards.create("zone", "race-1", {});
    await ctx.shards.stop("race-1");
    return "ok";
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

fn manifest(wasm: &Path) -> AppManifest {
    let field = |name: &str| ManifestField {
        name: name.into(),
        field_type: "string".into(),
        ..Default::default()
    };
    let shard: ManifestShard = serde_json::from_value(serde_json::json!({
        "name": "zone",
        "wasm": wasm.to_string_lossy(),
        "idleShutdownSecs": 600,
        "onStop": "shardEnded",
    }))
    .unwrap();
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "shard-on-stop".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Ended".into(),
            fields: vec![field("shardId"), field("kind"), field("reason")],
            ..Default::default()
        }],
        shards: vec![shard],
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

fn post(port: u16, path: &str) -> (u16, String) {
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
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

#[test]
fn on_stop_runs_when_a_shard_is_stopped() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let wasm = zone_wasm();
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
    }
    let rt = Arc::new(Runtime::in_memory(manifest(&wasm)).unwrap());
    let port = free_port();
    let server_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(server_rt, port);
    });

    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            let (status, body) = post(port, "/api/fn/startAndStop");
            if status == 200 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "startAndStop failed: {status} {body}"
            );
        }
        assert!(Instant::now() < deadline, "server never came up");
        std::thread::sleep(Duration::from_millis(200));
    }

    let rows = loop {
        let rows = rt.list("Ended").unwrap();
        if !rows.is_empty() || Instant::now() > deadline {
            break rows;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["shardId"], "race-1");
    assert_eq!(rows[0]["kind"], "zone");
    assert_eq!(rows[0]["reason"], "stopped");
}
