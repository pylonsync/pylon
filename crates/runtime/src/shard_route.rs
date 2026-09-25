//! Routing a shard connection that reached a machine that does not run the
//! shard (see `shard_cluster`).
//!
//! On Fly the machine answers with a `fly-replay: instance=<id>` header and
//! Fly's proxy sends the request to the right machine: the client connects
//! there directly, with no extra hop for its frames. Elsewhere the machine
//! proxies the request over the machines' private addresses: the WebSocket
//! and the SSE stream are spliced byte for byte after the request head, and
//! an input is forwarded with its response relayed.
//!
//! A forwarded request carries `X-Pylon-Shard-Forwarded: <client ip>;<sig>`,
//! signed for the receiving machine over the IP, method, and URL. The
//! receiver serves it locally (it never forwards again) and counts it
//! against the client's IP, not the proxying machine's.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use tiny_http::{Header, Request, Response};

use crate::shard_cluster::{self, FORWARDED_HEADER};

/// Largest input body forwarded.
const MAX_BODY: u64 = 1 << 20;

/// The shard a request is for, when it must be served where the shard
/// runs: the WebSocket (`/shard?shard=<id>`), the SSE stream
/// (`/api/shards/<id>/connect`), and inputs (`/api/shards/<id>/input`).
pub fn shard_of(url: &str) -> Option<String> {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    if path == "/shard" || path == "/" {
        return query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == "shard").then(|| decode(v)).filter(|v| !v.is_empty())
        });
    }
    let rest = path.strip_prefix("/api/shards/")?;
    let id = rest
        .strip_suffix("/connect")
        .or_else(|| rest.strip_suffix("/input"))?;
    (!id.is_empty() && !id.contains('/')).then(|| decode(id))
}

/// True for the SSE stream, which stays open.
pub fn is_stream(url: &str) -> bool {
    url.split('?')
        .next()
        .is_some_and(|p| p.starts_with("/api/shards/") && p.ends_with("/connect"))
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn forward_payload(ip: &str, method: &str, url: &str) -> Vec<u8> {
    format!("{ip}\n{method}\n{url}").into_bytes()
}

/// The `X-Pylon-Shard-Forwarded` value for a request from `ip` passed to
/// machine `to`. None before this machine joined the directory.
pub fn forwarded_value(ip: &str, method: &str, url: &str, to: &str) -> Option<String> {
    let sig = shard_cluster::sign(to, &forward_payload(ip, method, url))?;
    Some(format!("{ip};{sig}"))
}

/// The client IP a machine that forwarded this request vouched for, when
/// its header is present, signed for machine `me`, and not seen before.
pub fn forwarded_client_ip(request: &Request, me: Option<&str>) -> Option<String> {
    let me = me?;
    let value = request
        .headers()
        .iter()
        .find(|h| h.field.equiv(FORWARDED_HEADER))?
        .value
        .as_str();
    let (ip, sig) = value.split_once(';')?;
    ip.parse::<std::net::IpAddr>().ok()?;
    let payload = forward_payload(ip, request.method().as_str(), request.url());
    shard_cluster::verify(me, sig, &payload).ok()?;
    Some(ip.to_string())
}

fn json_response(status: u16, code: &str, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::json!({ "error": { "code": code, "message": message } });
    Response::from_string(body.to_string())
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

/// Where a request goes.
pub struct Target<'a> {
    pub machine_id: &'a str,
    pub address: Option<&'a str>,
    /// The Fly instance to replay on, when both machines run on Fly.
    pub fly_replay: Option<&'a str>,
}

/// A slot the connection holds for its life (a per-IP connection slot, a
/// stream slot).
pub type Slot = Box<dyn Send>;

/// Send `request` to the machine that runs its shard, and answer the
/// client. `client_ip` is the address the request came from; `slot` is
/// held for the life of a WebSocket or stream. Returns the status the
/// client got.
pub fn route(request: Request, target: Target<'_>, client_ip: &str, slot: Option<Slot>) -> u16 {
    if let Some(instance) = target.fly_replay {
        // Fly's proxy replays the request on that machine.
        let response = Response::from_string("").with_status_code(200).with_header(
            Header::from_bytes("fly-replay", format!("instance={instance}").as_bytes()).unwrap(),
        );
        let _ = request.respond(response);
        return 200;
    }
    let Some(address) = target.address else {
        let _ = request.respond(json_response(
            503,
            "SHARD_MACHINE_UNREACHABLE",
            &format!(
                "the shard runs on machine {}, which has no address (set PYLON_SHARD_ADVERTISE_URL)",
                target.machine_id
            ),
        ));
        return 503;
    };
    let is_upgrade = request
        .headers()
        .iter()
        .any(|h| h.field.equiv("Upgrade") && h.value.as_str().eq_ignore_ascii_case("websocket"));
    if is_upgrade {
        proxy_websocket(request, target.machine_id, address, client_ip, slot)
    } else if is_stream(request.url()) {
        proxy_stream(request, target.machine_id, address, client_ip, slot)
    } else {
        forward_http(request, target.machine_id, address, client_ip)
    }
}

/// `host:port` from `http://host:port`.
pub fn authority(address: &str) -> Option<&str> {
    address
        .strip_prefix("http://")
        .map(|a| a.split('/').next().unwrap_or(a))
}

/// Headers passed on to the machine that runs the shard: the client's,
/// minus hop-by-hop ones and any forwarding header it sent.
fn passed_headers(request: &Request) -> Vec<(String, String)> {
    request
        .headers()
        .iter()
        .filter(|h| {
            let f = h.field.as_str().as_str();
            !(f.eq_ignore_ascii_case("host")
                || f.eq_ignore_ascii_case("content-length")
                || f.eq_ignore_ascii_case("transfer-encoding")
                || f.eq_ignore_ascii_case(FORWARDED_HEADER))
        })
        .map(|h| (h.field.as_str().to_string(), h.value.as_str().to_string()))
        .collect()
}

/// Connect to `address` and send the request head, with the client's
/// headers and the signed forwarding header.
fn open_upstream(
    request: &Request,
    machine_id: &str,
    address: &str,
    client_ip: &str,
    extra: &str,
) -> Result<TcpStream, String> {
    let url = request.url();
    let method = request.method().as_str();
    let host = authority(address).ok_or("the machine address is not http://")?;
    let addr = host
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("the machine address does not resolve")?;
    let mut stream =
        TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    let forwarded = forwarded_value(client_ip, method, url, machine_id)
        .ok_or("this machine is not in the shard directory")?;
    let mut head = format!("{method} {url} HTTP/1.1\r\nHost: {host}\r\n");
    for (k, v) in passed_headers(request) {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("{FORWARDED_HEADER}: {forwarded}\r\n{extra}\r\n"));
    stream
        .write_all(head.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(stream)
}

/// The machine's socket after its response head: the status, its headers,
/// and any bytes it sent after the head.
type Upstream = (TcpStream, u16, Vec<(String, String)>, Vec<u8>);

fn read_head(mut stream: TcpStream) -> Result<Upstream, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 16 * 1024 {
            return Err("the response head is too large".into());
        }
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("the machine closed the connection".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or("bad status line")?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let _ = stream.set_read_timeout(None);
    Ok((stream, status, headers, buf[end..].to_vec()))
}

fn unreachable(request: Request, address: &str, e: &str) -> u16 {
    tracing::warn!("[shards] proxy to {address} failed: {e}");
    let _ = request.respond(json_response(
        502,
        "SHARD_MACHINE_UNREACHABLE",
        "the machine that runs the shard did not answer",
    ));
    502
}

fn proxy_websocket(
    request: Request,
    machine_id: &str,
    address: &str,
    client_ip: &str,
    slot: Option<Slot>,
) -> u16 {
    let upstream = open_upstream(&request, machine_id, address, client_ip, "").and_then(read_head);
    let (upstream, status, headers, early) = match upstream {
        Ok(v) => v,
        Err(e) => return unreachable(request, address, &e),
    };
    if status != 101 {
        // The machine refused (auth, unknown shard): pass its answer on.
        let mut response = Response::from_data(early).with_status_code(status);
        for (k, v) in headers {
            if k.eq_ignore_ascii_case("content-type") {
                if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                    response = response.with_header(h);
                }
            }
        }
        let _ = request.respond(response);
        return status;
    }
    let mut response = Response::empty(101);
    for (k, v) in &headers {
        let keep = [
            "upgrade",
            "connection",
            "sec-websocket-accept",
            "sec-websocket-protocol",
        ]
        .iter()
        .any(|n| k.eq_ignore_ascii_case(n));
        if keep {
            if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                response = response.with_header(h);
            }
        }
    }
    match request.upgrade_detached("websocket", response) {
        Ok(client) => {
            crate::shard_ws::splice(client, upstream, early, slot);
            101
        }
        Err(request) => {
            let _ = request.respond(json_response(
                501,
                "SHARD_UPGRADE_UNSUPPORTED",
                "this connection cannot be proxied (TLS terminated by pylon)",
            ));
            501
        }
    }
}

/// The SSE stream: the machine's whole response, head and body, is copied
/// to the client on a thread that holds the stream slot, so a long stream
/// never holds a request worker.
fn proxy_stream(
    request: Request,
    machine_id: &str,
    address: &str,
    client_ip: &str,
    slot: Option<Slot>,
) -> u16 {
    let upstream = match open_upstream(
        &request,
        machine_id,
        address,
        client_ip,
        "Connection: close\r\n",
    ) {
        Ok(s) => s,
        Err(e) => return unreachable(request, address, &e),
    };
    let mut writer = request.into_writer();
    let _ = std::thread::Builder::new()
        .name("pylon-shard-stream-proxy".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            let _slot = slot;
            let mut upstream = upstream;
            let mut buf = [0u8; 16 * 1024];
            // Copy and flush each read: SSE events must not wait in a buffer.
            loop {
                match upstream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if writer.write_all(&buf[..n]).is_err() || writer.flush().is_err() {
                            break;
                        }
                    }
                }
            }
        });
    200
}

fn forward_http(mut request: Request, machine_id: &str, address: &str, client_ip: &str) -> u16 {
    let url = request.url().to_string();
    let method = request.method().as_str().to_string();
    let mut body = Vec::new();
    if request
        .as_reader()
        .take(MAX_BODY + 1)
        .read_to_end(&mut body)
        .is_err()
    {
        let _ = request.respond(json_response(400, "BAD_REQUEST", "could not read the body"));
        return 400;
    }
    if body.len() as u64 > MAX_BODY {
        let _ = request.respond(json_response(
            413,
            "PAYLOAD_TOO_LARGE",
            "a shard input is at most 1 MiB",
        ));
        return 413;
    }
    let Some(forwarded) = forwarded_value(client_ip, &method, &url, machine_id) else {
        return unreachable(
            request,
            address,
            "this machine is not in the shard directory",
        );
    };
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .redirects(0)
        .build();
    let mut call = agent
        .request(&method, &format!("{address}{url}"))
        .set(FORWARDED_HEADER, &forwarded);
    for (k, v) in passed_headers(&request) {
        if !k.eq_ignore_ascii_case("connection") {
            call = call.set(&k, &v);
        }
    }
    let upstream = match call.send_bytes(&body) {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => return unreachable(request, address, &e.to_string()),
    };
    let status = upstream.status();
    let content_type = upstream.header("content-type").map(str::to_string);
    let mut text = Vec::new();
    let _ = upstream.into_reader().take(MAX_BODY).read_to_end(&mut text);
    let mut response = Response::from_data(text).with_status_code(status);
    if let Some(ct) = content_type {
        if let Ok(h) = Header::from_bytes("Content-Type", ct.as_bytes()) {
            response = response.with_header(h);
        }
    }
    let _ = request.respond(response);
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shard_of_a_request() {
        assert_eq!(
            shard_of("/shard?shard=zone-1&sid=u").as_deref(),
            Some("zone-1")
        );
        assert_eq!(
            shard_of("/shard?sid=u&shard=match%3A42&v=2").as_deref(),
            Some("match:42")
        );
        assert_eq!(
            shard_of("/?shard=zone&sid=u").as_deref(),
            Some("zone"),
            "the shard port"
        );
        assert_eq!(shard_of("/shard?sid=u"), None);
        assert_eq!(shard_of("/shard?shard="), None);
        assert_eq!(
            shard_of("/api/shards/zone/connect?sid=u").as_deref(),
            Some("zone")
        );
        assert_eq!(shard_of("/api/shards/zone/input").as_deref(), Some("zone"));
        assert_eq!(
            shard_of("/api/shards/zone"),
            None,
            "the admin detail runs anywhere"
        );
        assert_eq!(shard_of("/api/shards/zone/stop"), None);
        assert_eq!(shard_of("/api/shards/a/b/input"), None);
        assert_eq!(shard_of("/api/other"), None);
        assert!(is_stream("/api/shards/zone/connect?sid=u"));
        assert!(!is_stream("/api/shards/zone/input"));
    }

    #[test]
    fn authority_of_an_address() {
        assert_eq!(authority("http://10.0.0.2:4321"), Some("10.0.0.2:4321"));
        assert_eq!(authority("http://[fdaa::3]:8080"), Some("[fdaa::3]:8080"));
        assert_eq!(authority("https://x"), None);
    }
}
