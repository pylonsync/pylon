//! `pylon dev --host 0.0.0.0`: other devices can connect, but dev shortcuts
//! stay limited to this machine.
//!
//! The server runs with `PYLON_DEV_MODE=1` and `PYLON_HOST=0.0.0.0`. Requests
//! go to this machine's network address, so the server sees a non-loopback
//! peer, the same as a phone on the Wi-Fi. Skipped when the machine has no
//! network address.

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
    std::env::temp_dir().join(format!("pylon-dev-lan-{}", std::process::id()))
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
            std::env::set_var("PYLON_HOST", "0.0.0.0");
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
    body: String,
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
        body: body.to_string(),
    }
}

/// The server's network address, or `None` (test skipped) when the machine
/// has none.
fn lan(port: u16) -> Option<SocketAddr> {
    lan_ip().map(|ip| SocketAddr::new(ip, port))
}

#[test]
fn network_callers_get_no_dev_shortcuts() {
    let port = start();
    let Some(addr) = lan(port) else {
        eprintln!("no network address; skipping");
        return;
    };
    let host = addr.to_string();
    let origin = format!("http://{host}");

    // The server is reachable from the network.
    let health = send(addr, "GET", "/health", &[("Host", &host)], "");
    assert_eq!(health.status, 200, "{}", health.body);

    let mint = send(
        addr,
        "POST",
        "/api/auth/session",
        &[("Host", &host), ("Origin", &origin)],
        r#"{"user_id":"owner"}"#,
    );
    assert_eq!(
        mint.status, 403,
        "session mint from the network: {}",
        mint.body
    );

    let code = send(
        addr,
        "POST",
        "/api/auth/magic/send",
        &[("Host", &host), ("Origin", &origin)],
        r#"{"email":"phone@example.com"}"#,
    );
    assert!(
        !code.body.contains("dev_code"),
        "dev_code over the network: {}",
        code.body
    );

    let phone = send(
        addr,
        "POST",
        "/api/auth/phone/send-code",
        &[("Host", &host), ("Origin", &origin)],
        r#"{"phone":"+15551234567"}"#,
    );
    assert!(
        !phone.body.contains("dev_code"),
        "phone code over the network: {}",
        phone.body
    );

    let ticket = send(
        addr,
        "POST",
        "/admin/studio-ticket",
        &[("Host", &host), ("Origin", &origin)],
        "{}",
    );
    assert!(
        ticket.status == 401 || ticket.status == 403,
        "studio ticket from the network: {} {}",
        ticket.status,
        ticket.body
    );

    let write = send(
        addr,
        "POST",
        "/_pylon/dev/files/functions/pwn.ts",
        &[("Host", &host), ("Origin", &origin)],
        "export default 1;",
    );
    assert_eq!(
        write.status, 403,
        "file write from the network: {}",
        write.body
    );
    assert!(!watch_dir().join("functions/pwn.ts").exists());

    // A page opened by network address may call its own API (same origin).
    let me = send(
        addr,
        "POST",
        "/api/auth/guest",
        &[("Host", &host), ("Origin", &origin)],
        "",
    );
    assert!(
        me.status == 200 || me.status == 201,
        "same-origin POST from a LAN page: {} {}",
        me.status,
        me.body
    );
}

#[test]
fn this_machine_still_gets_dev_shortcuts() {
    let port = start();
    let host = format!("localhost:{port}");
    let mint = send(
        ([127, 0, 0, 1], port).into(),
        "POST",
        "/api/auth/session",
        &[("Host", &host)],
        r#"{"user_id":"owner"}"#,
    );
    assert_eq!(mint.status, 201, "{}", mint.body);
}
