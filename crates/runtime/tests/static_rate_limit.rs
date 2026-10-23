//! Static files do not count against the per-IP rate limiter.
//!
//! A page load fetches dozens of chunks. With the limiter counting them,
//! a production page got 429s on its own scripts after a burst of about
//! 50. Here the limit is 5 requests per minute: 30 chunk requests all
//! answer 200, and API requests are still limited.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::AppManifest;
use pylon_runtime::Runtime;

fn manifest() -> AppManifest {
    serde_json::from_value(serde_json::json!({
        "manifest_version": 1,
        "name": "static-rate-limit",
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
        if (0..4).all(|o| pylon_runtime::listen::port_is_free(base + o)) {
            return base;
        }
    }
    panic!("no free port block");
}

fn get_status(port: u16, path: &str) -> u16 {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok();
    raw.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

#[test]
fn static_files_skip_the_rate_limiter_and_api_requests_do_not() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "pylon-static-rl-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let dist = dir.join("dist");
    std::fs::create_dir_all(dist.join("assets")).unwrap();
    std::fs::write(dist.join("index.html"), "<!doctype html><title>t</title>").unwrap();
    std::fs::write(dist.join("assets/app-abcd1234.js"), "console.log(1);").unwrap();
    // SAFETY: set before the server thread starts; this binary has one test.
    unsafe {
        std::env::set_var("PYLON_FRONTEND_DIR", &dist);
        std::env::set_var("PYLON_RATE_LIMIT_MAX", "5");
        std::env::set_var("PYLON_RATE_LIMIT_WINDOW", "60");
        std::env::set_var("PYLON_CORS_ORIGIN", "http://127.0.0.1");
        std::env::set_var("PYLON_SESSION_IN_MEMORY", "1");
    }
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let port = free_port();
    std::thread::spawn(move || {
        if let Err(e) = pylon_runtime::server::start(rt, port) {
            eprintln!("server failed to start: {e}");
        }
    });
    // /health answers before the rate limiter, so polling it spends none of
    // the budget.
    let deadline = Instant::now() + Duration::from_secs(60);
    while TcpStream::connect(("127.0.0.1", port)).is_err() || get_status(port, "/health") != 200 {
        assert!(Instant::now() < deadline, "server never came up");
        std::thread::sleep(Duration::from_millis(200));
    }

    for i in 0..30 {
        assert_eq!(
            get_status(port, "/assets/app-abcd1234.js"),
            200,
            "chunk request {i} was limited"
        );
    }

    let statuses: Vec<u16> = (0..10).map(|_| get_status(port, "/api/auth/me")).collect();
    assert!(
        statuses.contains(&429),
        "API requests past the limit must still get 429: {statuses:?}"
    );
}
