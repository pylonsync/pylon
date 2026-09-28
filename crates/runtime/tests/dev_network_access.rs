//! `pylon dev` must not hand owner-level access to the network.
//!
//! This binary runs the server the way `pylon dev` does: `PYLON_DEV_MODE=1`
//! and `PYLON_HOST=localhost`. It checks that
//!
//! - every port listens on loopback only,
//! - dev shortcuts (session mint, `dev_code`) refuse a request whose `Host`
//!   is not this machine (DNS rebinding, tunnels),
//! - CORS never reflects a foreign origin,
//! - the dev file-write API refuses foreign pages and non-local hosts.
//!
//! `dev_network_access_lan.rs` covers a server listening on every interface
//! and a caller on another address.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::*;
use pylon_runtime::Runtime;

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: MANIFEST_VERSION,
        name: "dev-network-access".into(),
        version: "0.1.0".into(),
        ..Default::default()
    }
}

/// Watched dir for the file-write API, shared by the whole binary.
fn watch_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pylon-dev-net-{}", std::process::id()))
}

fn free_port_block() -> u16 {
    for _ in 0..200 {
        let base = 20_000 + rand::random::<u16>() % 8_000;
        if (0..4).all(|o| pylon_runtime::listen::port_is_free(base + o)) {
            return base;
        }
    }
    panic!("no free port block");
}

/// Whether `addr` accepts a connection within 5 seconds.
fn listening(addr: (&str, u16)) -> bool {
    for _ in 0..100 {
        if TcpStream::connect(addr).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn start() -> u16 {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| {
        std::fs::create_dir_all(watch_dir()).unwrap();
        // A static frontend dir turns on the frontend dispatcher, which
        // serves the /_pylon/dev/* endpoints.
        let web = watch_dir().join("dist");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), "<!doctype html><p>app</p>").unwrap();
        // SAFETY: once per binary, before any server thread starts.
        unsafe {
            std::env::set_var("PYLON_FRONTEND_DIR", &web);
            std::env::set_var("PYLON_DEV_MODE", "1");
            std::env::set_var("PYLON_HOST", "localhost");
            std::env::set_var("PYLON_DEV_WATCH_DIR", watch_dir());
            std::env::remove_var("PYLON_DEV_TRUST_REMOTE");
            std::env::remove_var("PYLON_DEV_FILE_API_TOKEN");
            std::env::remove_var("PYLON_CORS_ORIGIN");
            std::env::remove_var("PYLON_ADMIN_TOKEN");
        }
    });
    let port = free_port_block();
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return port;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not start on {port}");
}

/// A non-loopback address of this machine, if it has one. No packets are
/// sent: connecting a UDP socket only picks the outgoing interface.
fn lan_ip() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

struct Resp {
    status: u16,
    headers: String,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<String> {
        self.headers.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    }
}

fn send(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Resp {
    let mut req = format!("{method} {path} HTTP/1.1\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!(
        "Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ));
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Resp {
        status,
        headers: head.to_string(),
        body: body.to_string(),
    }
}

fn local(port: u16) -> SocketAddr {
    ([127, 0, 0, 1], port).into()
}

#[test]
fn every_port_listens_on_loopback_only() {
    let port = start();
    // HTTP and WebSocket answer on both loopback families. `start` waits
    // only for the HTTP port; the WebSocket listener binds a moment later.
    for p in [port, port + 1] {
        assert!(listening(("127.0.0.1", p)), "127.0.0.1:{p}");
        if std::net::TcpListener::bind("[::1]:0").is_ok() {
            assert!(listening(("::1", p)), "[::1]:{p}");
        }
    }
    // Nothing answers on the machine's network address.
    if let Some(ip) = lan_ip() {
        for p in [port, port + 1, port + 2] {
            let r = TcpStream::connect_timeout(&SocketAddr::new(ip, p), Duration::from_secs(2));
            assert!(r.is_err(), "{ip}:{p} accepted a connection");
        }
    }
}

#[test]
fn session_mint_needs_a_local_host_header() {
    let port = start();
    let host = format!("127.0.0.1:{port}");
    let ok = send(
        local(port),
        "POST",
        "/api/auth/session",
        &[("Host", &host)],
        r#"{"user_id":"owner"}"#,
    );
    assert_eq!(ok.status, 201, "local mint: {}", ok.body);

    // DNS rebinding: the browser connects to 127.0.0.1 but believes it is
    // talking to evil.example, and says so in Host.
    let evil_host = format!("evil.example:{port}");
    let rebound = send(
        local(port),
        "POST",
        "/api/auth/session",
        &[
            ("Host", &evil_host),
            ("Origin", &format!("http://{evil_host}")),
        ],
        r#"{"user_id":"owner"}"#,
    );
    assert_eq!(rebound.status, 403, "rebound mint: {}", rebound.body);
    assert!(!rebound.body.contains("token"), "{}", rebound.body);
}

#[test]
fn dev_code_needs_a_local_host_header() {
    let port = start();
    let host = format!("localhost:{port}");
    let ok = send(
        local(port),
        "POST",
        "/api/auth/magic/send",
        &[("Host", &host)],
        r#"{"email":"dev-local@example.com"}"#,
    );
    assert_eq!(ok.status, 200, "{}", ok.body);
    assert!(
        ok.body.contains("dev_code"),
        "local caller gets the code: {}",
        ok.body
    );

    let tunnel = send(
        local(port),
        "POST",
        "/api/auth/magic/send",
        &[("Host", "abc123.ngrok.io")],
        r#"{"email":"dev-tunnel@example.com"}"#,
    );
    assert!(
        !tunnel.body.contains("dev_code"),
        "tunnel caller got the code: {}",
        tunnel.body
    );
}

#[test]
fn cors_never_reflects_a_foreign_origin() {
    let port = start();
    let host = format!("127.0.0.1:{port}");
    let foreign = send(
        local(port),
        "GET",
        "/api/auth/me",
        &[("Host", &host), ("Origin", "https://evil.example")],
        "",
    );
    let acao = foreign
        .header("Access-Control-Allow-Origin")
        .unwrap_or_default();
    assert_ne!(acao, "https://evil.example", "foreign origin reflected");
    if acao == "*" {
        assert_ne!(
            foreign
                .header("Access-Control-Allow-Credentials")
                .as_deref(),
            Some("true"),
            "wildcard with credentials"
        );
    }

    let dev_page = send(
        local(port),
        "GET",
        "/api/auth/me",
        &[("Host", &host), ("Origin", "http://localhost:3000")],
        "",
    );
    assert_eq!(
        dev_page.header("Access-Control-Allow-Origin").as_deref(),
        Some("http://localhost:3000")
    );
    assert_eq!(
        dev_page
            .header("Access-Control-Allow-Credentials")
            .as_deref(),
        Some("true")
    );
}

#[test]
fn file_api_refuses_foreign_pages_and_hosts() {
    let port = start();
    let host = format!("localhost:{port}");

    // A website open in the developer's browser posting a function file.
    let foreign = send(
        local(port),
        "POST",
        "/_pylon/dev/files/functions/evil.ts",
        &[("Host", &host), ("Origin", "https://evil.example")],
        "export default 1;",
    );
    assert!(
        foreign.status == 403,
        "foreign page: {} {}",
        foreign.status,
        foreign.body
    );
    assert!(!watch_dir().join("functions/evil.ts").exists());

    let rebound = send(
        local(port),
        "POST",
        "/_pylon/dev/files/rebound.txt",
        &[("Host", "evil.example")],
        "x",
    );
    assert_eq!(rebound.status, 403, "{}", rebound.body);
    assert!(!watch_dir().join("rebound.txt").exists());

    // The local tool (no Origin, localhost Host) still writes.
    let ok = send(
        local(port),
        "POST",
        "/_pylon/dev/files/notes.txt",
        &[("Host", &host)],
        "hello",
    );
    assert_eq!(ok.status, 200, "{}", ok.body);
    assert_eq!(
        std::fs::read_to_string(watch_dir().join("notes.txt")).unwrap(),
        "hello"
    );
}

/// A website open in the developer's browser sends `no-cors` requests to
/// localhost: loopback peer, `Host: localhost`, but its own Origin. It must
/// not reach the open-in-dev /admin routes, which run before the CSRF gate.
#[test]
fn a_foreign_page_cannot_use_dev_admin_routes() {
    let port = start();
    let host = format!("localhost:{port}");
    let forged = send(
        local(port),
        "POST",
        "/admin/operators",
        &[("Host", &host), ("Origin", "https://evil.example")],
        r#"{"username":"pwned","password":"correct horse battery staple"}"#,
    );
    assert!(
        forged.status == 401 || forged.status == 403,
        "operator created from a foreign page: {} {}",
        forged.status,
        forged.body
    );

    let local_tool = send(
        local(port),
        "GET",
        "/admin/operators",
        &[("Host", &host)],
        "",
    );
    assert_eq!(local_tool.status, 200, "{}", local_tool.body);
    assert!(!local_tool.body.contains("pwned"), "{}", local_tool.body);
}

/// DNS rebinding: same-origin to the attacker's own name, so the dev CSRF
/// rule lets it through, but it must not reset the dev database.
#[test]
fn rebinding_cannot_reset_the_dev_database() {
    let port = start();
    let host = format!("evil.example:{port}");
    let r = send(
        local(port),
        "POST",
        "/api/__test__/reset",
        &[("Host", &host), ("Origin", &format!("http://{host}"))],
        "",
    );
    assert_eq!(r.status, 403, "{}", r.body);
}
