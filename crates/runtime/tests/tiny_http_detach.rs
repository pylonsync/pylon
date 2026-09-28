//! The vendored tiny_http's detach patch takes a request's connection out
//! of tiny_http. It must take THAT connection, even after tiny_http has
//! finished with the connection's own file descriptors.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// tiny_http ends an HTTP/1.0 connection as soon as it has read the
/// request and closes its descriptors while the request is still being
/// handled. The detach handle held only the raw descriptor number, so a
/// connection accepted in between could get that number, and the detach
/// took over the other client's socket: the first client's response went
/// to the second client.
#[test]
fn a_detach_after_the_connection_ended_takes_the_right_socket() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = server.server_addr().to_ip().unwrap();

    let mut first = TcpStream::connect(addr).unwrap();
    first
        .write_all(b"GET /first HTTP/1.0\r\nHost: x\r\n\r\n")
        .unwrap();
    let request = server.recv().unwrap();
    assert_eq!(request.url(), "/first");
    // Let tiny_http finish the HTTP/1.0 connection and close its fds.
    std::thread::sleep(Duration::from_millis(200));

    // Other connections take the freed descriptor numbers.
    let mut others: Vec<TcpStream> = (0..4)
        .map(|_| {
            let s = TcpStream::connect(addr).unwrap();
            s.set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            s
        })
        .collect();
    std::thread::sleep(Duration::from_millis(100));

    let mut detached = request
        .into_detached_stream()
        .unwrap_or_else(|_| panic!("plain TCP detaches"));
    detached
        .write_all(b"HTTP/1.0 200 OK\r\nConnection: close\r\n\r\nfor-first")
        .unwrap();
    drop(detached);

    first
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut got = Vec::new();
    let _ = first.read_to_end(&mut got);
    assert!(
        String::from_utf8_lossy(&got).contains("for-first"),
        "the first client did not get its response: {:?}",
        String::from_utf8_lossy(&got)
    );
    for other in &mut others {
        let mut buf = [0u8; 256];
        if let Ok(n) = other.read(&mut buf) {
            assert!(
                !String::from_utf8_lossy(&buf[..n]).contains("for-first"),
                "another client got the first client's response"
            );
        }
    }
}
