//! The C ABI against a local WebTransport server that echoes its stream and
//! its datagrams, with a self-signed certificate the client pins by hash.

use std::ffi::CString;
use std::time::{Duration, Instant};

use pylon_shard_client::*;
use wtransport::{Endpoint, Identity, ServerConfig, VarInt};

struct Server {
    port: u16,
    hash: [u8; 32],
    _rt: tokio::runtime::Runtime,
}

/// A server on a free port. After `close_after` echoed stream bytes, it
/// closes the session with code 3 and reason "again".
fn server(close_after: Option<usize>) -> Server {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let identity = Identity::self_signed(["localhost", "127.0.0.1"]).unwrap();
    let hash = *identity.certificate_chain().as_slice()[0].hash().as_ref();
    let config = ServerConfig::builder()
        .with_bind_default(0)
        .with_identity(identity)
        .build();
    let endpoint = rt.block_on(async { Endpoint::server(config) }).unwrap();
    let port = endpoint.local_addr().unwrap().port();
    rt.spawn(async move {
        loop {
            let incoming = endpoint.accept().await;
            tokio::spawn(async move {
                let Ok(request) = incoming.await else { return };
                let Ok(conn) = request.accept().await else {
                    return;
                };
                let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                    return;
                };
                let datagrams = conn.clone();
                tokio::spawn(async move {
                    while let Ok(d) = datagrams.receive_datagram().await {
                        let _ = datagrams.send_datagram(d.payload());
                    }
                });
                let mut buf = vec![0u8; 4096];
                let mut echoed = 0;
                while let Ok(Some(n)) = recv.read(&mut buf).await {
                    if send.write_all(&buf[..n]).await.is_err() {
                        return;
                    }
                    echoed += n;
                    if close_after.is_some_and(|limit| echoed >= limit) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        conn.close(VarInt::from_u32(3), b"again");
                        return;
                    }
                }
            });
        }
    });
    Server {
        port,
        hash,
        _rt: rt,
    }
}

fn connect(url: &str, hashes: &[[u8; 32]]) -> u64 {
    let url = CString::new(url).unwrap();
    let flat: Vec<u8> = hashes.iter().flatten().copied().collect();
    unsafe { pylon_wt_connect(url.as_ptr(), flat.as_ptr(), hashes.len()) }
}

fn wait_for(handle: u64, state: i32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = pylon_wt_state(handle);
        if s == state {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "state {s}, wanted {state}: {}",
            error(handle)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn error(handle: u64) -> String {
    let mut buf = vec![0u8; 512];
    let n = unsafe { pylon_wt_error(handle, buf.as_mut_ptr(), buf.len()) };
    String::from_utf8_lossy(&buf[..n.max(0) as usize]).into_owned()
}

#[test]
fn the_stream_and_datagrams_echo_through_the_c_abi() {
    let s = server(None);
    let h = connect(&format!("https://127.0.0.1:{}/shard", s.port), &[s.hash]);
    assert_ne!(h, 0);
    wait_for(h, STATE_OPEN);
    assert!(
        pylon_wt_max_datagram_size(h) > 1000,
        "{}",
        pylon_wt_max_datagram_size(h)
    );

    // The stream: bytes come back in order, across several writes.
    let message: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    for chunk in message.chunks(3000) {
        assert_eq!(
            unsafe { pylon_wt_stream_write(h, chunk.as_ptr(), chunk.len()) },
            0
        );
    }
    let mut got = Vec::new();
    let mut buf = vec![0u8; 1500];
    let deadline = Instant::now() + Duration::from_secs(10);
    while got.len() < message.len() {
        let n = unsafe { pylon_wt_stream_read(h, buf.as_mut_ptr(), buf.len()) };
        assert!(n >= 0, "stream read {n}");
        got.extend_from_slice(&buf[..n as usize]);
        assert!(
            Instant::now() < deadline,
            "only {} of {} bytes back",
            got.len(),
            message.len()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(got, message);

    // Datagrams: each one comes back whole.
    let mut back = Vec::new();
    for round in 0..20u8 {
        let d = vec![round; 100 + round as usize];
        assert_eq!(unsafe { pylon_wt_send_datagram(h, d.as_ptr(), d.len()) }, 0);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut dbuf = vec![0u8; 2048];
    while back.len() < 15 && Instant::now() < deadline {
        let n = unsafe { pylon_wt_recv_datagram(h, dbuf.as_mut_ptr(), dbuf.len()) };
        if n >= 0 {
            back.push(dbuf[..n as usize].to_vec());
        } else {
            assert_eq!(n, ERR_NONE);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    // Loopback can still drop one; nearly all arrive, each intact.
    assert!(back.len() >= 15, "{} datagrams back", back.len());
    for d in &back {
        assert!(d.iter().all(|b| *b == d[0]) && d.len() == 100 + d[0] as usize);
    }

    // A buffer too small keeps the datagram queued. Drain the earlier ones first.
    std::thread::sleep(Duration::from_millis(100));
    while unsafe { pylon_wt_recv_datagram(h, dbuf.as_mut_ptr(), dbuf.len()) } >= 0 {}
    let big = vec![7u8; 900];
    assert_eq!(
        unsafe { pylon_wt_send_datagram(h, big.as_ptr(), big.len()) },
        0
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut small = vec![0u8; 10];
    loop {
        let n = unsafe { pylon_wt_recv_datagram(h, small.as_mut_ptr(), small.len()) };
        if n == ERR_BUFFER_TOO_SMALL {
            break;
        }
        assert!(Instant::now() < deadline, "no datagram came back: {n}");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        unsafe { pylon_wt_recv_datagram(h, dbuf.as_mut_ptr(), dbuf.len()) },
        900
    );

    assert_eq!(unsafe { pylon_wt_close(h, 0, std::ptr::null(), 0) }, 0);
    assert_eq!(pylon_wt_state(h), STATE_CLOSED);
    pylon_wt_free(h);
    assert_eq!(pylon_wt_state(h), ERR_UNKNOWN_HANDLE);
}

#[test]
fn a_server_close_reports_its_code_and_reason() {
    let s = server(Some(4));
    let h = connect(&format!("https://127.0.0.1:{}/shard", s.port), &[s.hash]);
    wait_for(h, STATE_OPEN);
    assert_eq!(unsafe { pylon_wt_stream_write(h, b"ping".as_ptr(), 4) }, 0);
    wait_for(h, STATE_CLOSED);
    let mut code = 0u32;
    let mut reason = vec![0u8; 64];
    let n = unsafe { pylon_wt_close_info(h, &mut code, reason.as_mut_ptr(), reason.len()) };
    assert_eq!(code, 3);
    assert_eq!(&reason[..n as usize], b"again");
    // The bytes the server echoed before it closed are still readable.
    let mut buf = vec![0u8; 16];
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    loop {
        let n = unsafe { pylon_wt_stream_read(h, buf.as_mut_ptr(), buf.len()) };
        if n == ERR_STREAM_ENDED as i64 {
            break;
        }
        if n > 0 {
            got.extend_from_slice(&buf[..n as usize]);
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(got.is_empty() || got == b"ping", "{got:?}");
    pylon_wt_free(h);
}

#[test]
fn a_wrong_certificate_hash_fails_the_session() {
    let s = server(None);
    let h = connect(&format!("https://127.0.0.1:{}/shard", s.port), &[[0u8; 32]]);
    wait_for(h, STATE_FAILED);
    assert!(error(h).contains("connect"), "{}", error(h));
    pylon_wt_free(h);
}

#[test]
fn bad_arguments_and_unknown_handles_are_refused() {
    assert_eq!(connect("http://127.0.0.1:1/shard", &[]), 0, "plain http");
    assert_eq!(
        unsafe { pylon_wt_connect(std::ptr::null(), std::ptr::null(), 0) },
        0
    );
    let url = CString::new("https://127.0.0.1:1/").unwrap();
    assert_eq!(
        unsafe { pylon_wt_connect(url.as_ptr(), std::ptr::null(), 1) },
        0,
        "null hashes"
    );
    assert_eq!(pylon_wt_state(u64::MAX), ERR_UNKNOWN_HANDLE);
    assert_eq!(
        unsafe { pylon_wt_send_datagram(u64::MAX, std::ptr::null(), 0) },
        ERR_UNKNOWN_HANDLE
    );
    pylon_wt_free(u64::MAX);
    let v = unsafe { std::ffi::CStr::from_ptr(pylon_wt_version()) };
    assert_eq!(v.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn a_server_that_never_answers_fails_the_session_in_bounded_time() {
    // A bound socket that reads nothing: no handshake answer, and no ICMP
    // error to end the attempt early.
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();
    let started = Instant::now();
    let h = connect(&format!("https://127.0.0.1:{port}/shard"), &[[1u8; 32]]);
    assert_ne!(h, 0);
    let deadline = started + Duration::from_secs(20);
    while pylon_wt_state(h) == STATE_CONNECTING {
        assert!(Instant::now() < deadline, "still connecting after 20 s");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(pylon_wt_state(h), STATE_FAILED);
    assert!(error(h).contains("no answer"), "{}", error(h));
    assert!(
        started.elapsed() >= Duration::from_secs(9),
        "{:?}",
        started.elapsed()
    );
    pylon_wt_free(h);
    drop(silent);
}
