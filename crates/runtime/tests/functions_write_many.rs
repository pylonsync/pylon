//! `ctx.db.insertMany` / `ctx.db.updateMany` on SQLite, from a real
//! mutation: the rows land as `insert` / `update` one at a time would
//! store them (field defaults included), `updateMany` answers false for a
//! row that does not exist, and a malformed call is refused. The Postgres
//! path is in `postgres_write_round_trips.rs`.
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
        "pylon-write-many-{}-{}/functions",
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
    let body = r#"export default mutation({
  auth: "public",
  handler: async (ctx, args) => {
    try {
      if (args.op === "insert") return { ids: await ctx.db.insertMany("Lead", args.rows) };
      return { updated: await ctx.db.updateMany("Lead", args.rows) };
    } catch (e) {
      return { code: e.code ?? String(e) };
    }
  },
});"#;
    std::fs::write(
        fns.join("writeMany.ts"),
        format!("import {{ mutation }} from \"{define}\";\n{body}\n"),
    )
    .unwrap();
    fns
}

fn manifest() -> AppManifest {
    let field = |name: &str, field_type: &str, default: Option<serde_json::Value>| ManifestField {
        name: name.into(),
        field_type: field_type.into(),
        optional: false,
        default,
        ..Default::default()
    };
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "write-many".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Lead".into(),
            fields: vec![
                field("name", "string", None),
                field("stage", "string", Some(serde_json::json!("new"))),
                field("visits", "int", Some(serde_json::json!(0))),
            ],
            audit: true,
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

fn call(port: u16, body: &serde_json::Value) -> (u16, serde_json::Value) {
    let body = body.to_string();
    let req = format!(
        "POST /api/fn/writeMany HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
fn insert_many_and_update_many_write_rows_as_one_at_a_time() {
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
    }
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let port = free_port();
    let server_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(server_rt, port);
    });

    // The functions pool may still be loading: retry the first call.
    let insert = serde_json::json!({
        "op": "insert",
        "rows": [{ "name": "a" }, { "name": "b", "stage": "won", "visits": 3 }],
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let inserted = loop {
        let (status, body) = call(port, &insert);
        if status == 200 {
            break body;
        }
        assert!(Instant::now() < deadline, "never answered: {status} {body}");
        std::thread::sleep(Duration::from_millis(200));
    };
    let ids: Vec<String> = inserted["ids"]
        .as_array()
        .unwrap_or_else(|| panic!("{inserted}"))
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 2);
    let a = rt.get_by_id("Lead", &ids[0]).unwrap().expect("a");
    assert_eq!(
        (&a["name"], &a["stage"], &a["visits"]),
        (&"a".into(), &"new".into(), &0.into()),
        "{a}"
    );
    let b = rt.get_by_id("Lead", &ids[1]).unwrap().expect("b");
    assert_eq!(
        (&b["name"], &b["stage"], &b["visits"]),
        (&"b".into(), &"won".into(), &3.into()),
        "{b}"
    );

    let update = serde_json::json!({
        "op": "update",
        "rows": [
            { "id": ids[0], "data": { "stage": "contacted" } },
            { "id": "no-such-lead", "data": { "stage": "contacted" } },
            { "id": ids[1], "data": { "visits": 4 } },
        ],
    });
    let (status, body) = call(port, &update);
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["updated"],
        serde_json::json!([true, false, true]),
        "{body}"
    );
    let a = rt.get_by_id("Lead", &ids[0]).unwrap().expect("a");
    assert_eq!(
        (&a["stage"], &a["visits"]),
        (&"contacted".into(), &0.into())
    );
    let b = rt.get_by_id("Lead", &ids[1]).unwrap().expect("b");
    assert_eq!((&b["stage"], &b["visits"]), (&"won".into(), &4.into()));

    // A row without an id is refused, and nothing is written.
    let (status, body) = call(
        port,
        &serde_json::json!({
            "op": "update",
            "rows": [{ "data": { "stage": "lost" } }],
        }),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["code"], "INVALID_ARGS", "{body}");
    let (status, body) = call(port, &serde_json::json!({ "op": "insert", "rows": [1, 2] }));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["code"], "INVALID_ARGS", "{body}");
}
