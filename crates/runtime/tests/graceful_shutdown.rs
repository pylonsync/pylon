//! A shutdown closes the listener at once and still answers requests on
//! connections that were already open.
//!
//! Its own test binary: `request_shutdown` sets process-wide state.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::AppManifest;
use pylon_runtime::Runtime;

fn manifest() -> AppManifest {
    AppManifest {
        required_env: Vec::new(),
        build: Default::default(),
        shards: Vec::new(),
        manifest_version: 1,
        name: "graceful-shutdown".into(),
        version: "0.1.0".into(),
        entities: vec![],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

fn free_port() -> u16 {
    for _ in 0..200 {
        let base = 20_000 + rand::random::<u16>() % 8_000;
        if (0..4).all(|off| pylon_runtime::listen::port_is_free(base + off)) {
            return base;
        }
    }
    panic!("no free 4-port block");
}

/// Send `GET /health` on `stream` (keep-alive) and return the status code.
fn get_health(stream: &mut TcpStream) -> u16 {
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body).unwrap();
    status.split(' ').nth(1).unwrap().parse().unwrap()
}

#[test]
fn shutdown_refuses_new_connections_and_answers_open_ones() {
    // SAFETY: set before the server thread starts.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
    }
    let port = free_port();
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let server = std::thread::spawn(move || pylon_runtime::server::start(rt, port));

    // Wait for a served response, not only a bound port: the listener binds
    // before the request workers start.
    let addr = format!("127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut open = loop {
        if let Ok(mut s) = TcpStream::connect(&addr) {
            s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| get_health(&mut s)))
                .is_ok_and(|code| code == 200)
            {
                break s;
            }
        }
        if server.is_finished() {
            panic!("the server exited at boot: {:?}", server.join().unwrap());
        }
        assert!(
            Instant::now() < deadline,
            "the server never answered GET /health"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    open.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

    pylon_runtime::server::request_shutdown();

    // New connections are refused within a moment, not accepted and left
    // unanswered.
    let deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(&addr).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the listener still accepts after shutdown"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // A wake-up marker left in the request queue (here, from a second
    // shutdown request) is not mistaken for a quiet queue: the connection
    // that was already open still gets an answer.
    pylon_runtime::server::request_shutdown();
    assert_eq!(get_health(&mut open), 200);

    // With nothing more to serve, the server stops.
    drop(open);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !server.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the server did not stop after the drain"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
