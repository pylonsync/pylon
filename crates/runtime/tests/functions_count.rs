//! `ctx.db.count` from real functions on SQLite:
//!
//! - a query counts every row that matches a filter, past the row limit
//!   that `ctx.db.query` stops at;
//! - a mutation counts rows it inserted earlier in the same transaction.
//!
//! The Postgres statement is covered in `postgres_backend.rs`.
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

/// The repo root without Windows' `\\?\` prefix, which Bun cannot read in an import path.
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
    let fns = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "pylon-count-{}-{}/functions",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&fns).unwrap();
    let define = repo_root()
        .join("packages/functions/src/define.ts")
        .to_string_lossy()
        .replace('\\', "/");
    let count = r#"export default query({
  auth: "public",
  handler: async (ctx, args) => {
    try {
      return {
        count: await ctx.db.count("Lead", args.filter),
        all: await ctx.db.count("Lead"),
        queried: (await ctx.db.query("Lead", args.filter ?? {})).length,
      };
    } catch (e) {
      return { code: e.code ?? String(e) };
    }
  },
});"#;
    let add = r#"export default mutation({
  auth: "public",
  handler: async (ctx, args) => {
    const before = await ctx.db.count("Lead", { stage: "fresh" });
    await ctx.db.insert("Lead", { name: "x", stage: "fresh" });
    await ctx.db.insert("Lead", { name: "y", stage: "fresh" });
    return { before, after: await ctx.db.count("Lead", { stage: "fresh" }) };
  },
});"#;
    std::fs::write(
        fns.join("countLeads.ts"),
        format!("import {{ query }} from \"{define}\";\n{count}\n"),
    )
    .unwrap();
    std::fs::write(
        fns.join("addAndCount.ts"),
        format!("import {{ mutation }} from \"{define}\";\n{add}\n"),
    )
    .unwrap();
    fns
}

fn manifest() -> AppManifest {
    let field = |name: &str, optional: bool| ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional,
        ..Default::default()
    };
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "count".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Lead".into(),
            fields: vec![field("name", false), field("stage", true)],
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

fn call(port: u16, name: &str, body: &serde_json::Value) -> (u16, serde_json::Value) {
    let body = body.to_string();
    let req = format!(
        "POST /api/fn/{name} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return (0, serde_json::Value::Null);
    };
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
        .map(|(_, b)| b)
        .unwrap_or_default();
    (
        status,
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null),
    )
}

#[test]
fn count_is_exact_past_the_query_limit_and_sees_its_own_transaction() {
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
        // `ctx.db.query` stops at 5 rows, so a count that loaded rows would
        // answer 5.
        std::env::set_var("PYLON_QUERY_MAX_LIMIT", "5");
    }
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    for i in 0..12 {
        let stage = if i < 9 { Some("open") } else { None };
        rt.insert(
            "Lead",
            &serde_json::json!({ "name": format!("lead {i}"), "stage": stage }),
        )
        .unwrap();
    }
    let port = free_port();
    let server_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(server_rt, port);
    });

    // The functions pool may still be loading: retry the first call.
    let open = serde_json::json!({ "filter": { "stage": "open" } });
    let deadline = Instant::now() + Duration::from_secs(60);
    let counted = loop {
        let (status, body) = call(port, "countLeads", &open);
        if status == 200 {
            break body;
        }
        assert!(Instant::now() < deadline, "never answered: {status} {body}");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(counted["count"], 9, "{counted}");
    assert_eq!(counted["all"], 12, "{counted}");
    assert_eq!(counted["queried"], 5, "{counted}");

    let (status, body) = call(
        port,
        "countLeads",
        &serde_json::json!({ "filter": { "stage": null, "$limit": 1 } }),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["count"], 3, "{body}");

    let (status, body) = call(
        port,
        "countLeads",
        &serde_json::json!({ "filter": { "noSuchField": 1 } }),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["code"], "INVALID_COLUMN", "{body}");

    let (status, body) = call(port, "addAndCount", &serde_json::json!({}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["before"], 0, "{body}");
    assert_eq!(body["after"], 2, "{body}");
}
