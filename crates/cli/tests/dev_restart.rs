//! `pylon dev` survives the restart a manifest change triggers.
//!
//! Boots `pylon dev` on a minimal app, changes `app.ts` (a manifest change
//! re-execs the process) while requests are in flight, and checks that the
//! server answers again and serves the new function each time. Before the
//! fix, the outgoing image closed descriptors other threads still owned; a
//! debug build aborted ("owned file descriptor already closed") right after
//! printing "restarting", and nothing listened on the port again.
//!
//! Skipped (with a message) when `bun` is not on PATH.
//! Unix only: the restart there is an exec, and the test signals the
//! process with SIGTERM.
#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn bun_available() -> bool {
    Command::new("bun")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// `app.ts` printing a manifest with one entity per name in `entities`.
fn app_ts(entities: &[&str]) -> String {
    let entities: Vec<serde_json::Value> = entities
        .iter()
        .map(|name| {
            serde_json::json!({
                "name": name,
                "fields": [{ "name": "body", "type": "string", "optional": true, "unique": false }],
                "indexes": [],
                "relations": []
            })
        })
        .collect();
    let manifest = serde_json::json!({
        "manifest_version": 1,
        "name": "restart-test",
        "version": "0.1.0",
        "entities": entities,
        "routes": [],
        "queries": [],
        "actions": [],
        "policies": []
    });
    format!("console.log(JSON.stringify({manifest}));\n")
}

fn function_ts(value: &str) -> String {
    format!("export default {{ type: \"query\", auth: \"public\", handler: async () => \"{value}\" }};\n")
}

fn free_port() -> u16 {
    // `pylon dev` also binds port+1 (WebSocket) and port+2 (SSE).
    loop {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        if port < 65000 && (0..3).all(|off| pylon_runtime::listen::port_is_free(port + off)) {
            return port;
        }
    }
}

/// One HTTP request with a short timeout. `None` when nothing answered.
fn request(port: u16, method: &str, path: &str) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(500),
    )
    .ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let body = if method == "POST" { "{}" } else { "" };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    let status = raw.split_whitespace().nth(1)?.parse().ok()?;
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b.to_string())?;
    Some((status, body))
}

/// Wait until `/api/fn/<name>` answers `expected`.
fn wait_for_fn(port: u16, name: &str, expected: &str, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some((200, body)) = request(port, "POST", &format!("/api/fn/{name}")) {
            if body.contains(expected) {
                return;
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("pylon dev exited with {status} instead of coming back");
        }
        assert!(
            Instant::now() < deadline,
            "pylon dev did not serve {name} = {expected} within 60s"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn dev_server_comes_back_after_each_restart() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "pylon-dev-restart-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("functions")).unwrap();
    std::fs::write(dir.join("app.ts"), app_ts(&["Note"])).unwrap();
    std::fs::write(dir.join("functions/one.ts"), function_ts("one")).unwrap();

    let port = free_port();
    let log = std::fs::File::create(dir.join("dev.log")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_pylon"))
        .args(["dev", "--port", &port.to_string()])
        .current_dir(&dir)
        .env(
            "PYLON_FUNCTIONS_RUNTIME",
            repo_root().join("packages/functions/src/runtime.ts"),
        )
        .env("PYLON_FN_POOL_SIZE", "1")
        .env("PYLON_TELEMETRY", "0")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .expect("spawn pylon dev");
    let mut child = KillOnDrop(child);
    wait_for_fn(port, "one", "one", &mut child.0);

    for round in 1..=3 {
        // A new function and a manifest change: the manifest change makes
        // `pylon dev` re-exec itself.
        let name = format!("round{round}");
        std::fs::write(dir.join(format!("functions/{name}.ts")), function_ts(&name)).unwrap();
        let entities: Vec<String> = (0..=round).map(|i| format!("Entity{i}")).collect();
        let entities: Vec<&str> = entities.iter().map(String::as_str).collect();
        std::fs::write(dir.join("app.ts"), app_ts(&entities)).unwrap();

        // Requests in flight across the restart.
        let hammer = std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(3);
            while Instant::now() < until {
                let _ = request(port, "GET", "/health");
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        wait_for_fn(port, &name, &name, &mut child.0);
        hammer.join().unwrap();
    }

    // Every round's manifest change restarted the process, and the server
    // is up after the last one.
    let read_log = || std::fs::read_to_string(dir.join("dev.log")).unwrap_or_default();
    let deadline = Instant::now() + Duration::from_secs(30);
    while read_log().matches("restarting").count() < 3 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    wait_for_fn(port, "round3", "round3", &mut child.0);
    let log = read_log();
    assert!(
        log.matches("restarting").count() >= 3,
        "each manifest change should have restarted pylon dev:\n{log}"
    );
    assert!(
        !log.contains("IO Safety violation"),
        "the restart broke IO safety:\n{log}"
    );

    // One SIGTERM (what Ctrl-C sends, as SIGINT) ends the process once the
    // drain finishes. It used to drain and then keep running with no
    // listener until a second signal or kill -9.
    let pid = child.0.id() as libc::pid_t;
    // SAFETY: signalling our own child.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(Some(_)) = child.0.try_wait() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "pylon dev was still running 20s after SIGTERM:\n{}",
            read_log()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
