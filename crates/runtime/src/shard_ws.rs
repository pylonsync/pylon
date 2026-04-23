//! Bidirectional WebSocket server for real-time shards.
//!
//! Runs on its own port (typically `pylon_port + 3`). Each connection:
//!
//! 1. Parses the request path for `?shard=<id>&sid=<subscriber>`.
//! 2. Looks up the shard in the [`DynShardRegistry`].
//! 3. Runs the subscribe authorization hook.
//! 4. Registers a [`SnapshotSink`] that writes binary frames to the socket.
//! 5. Reads text/binary frames from the client and pushes them as inputs.
//! 6. Cleans up on disconnect.
//!
//! Each client gets its own dedicated thread. For larger deployments,
//! swap in an async runtime; for pylon's current scale, thread-per-conn
//! is simpler and fine.

use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pylon_auth::SessionStore;
use pylon_realtime::{DynShardRegistry, ShardAuth, ShardError, SubscriberId};
use tungstenite::{accept_hdr, handshake::server::Request, Message};

use crate::ip_limit::IpConnCounter;

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

/// Run a WebSocket server that accepts shard connections.
///
/// Blocking. Spawn on a background thread.
pub fn start_shard_ws_server(
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
    port: u16,
) {
    let listener = match TcpListener::bind(format!("0.0.0.0:{port}")) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[shard-ws] failed to bind port {port}: {e}");
            return;
        }
    };
    tracing::warn!("[shard-ws] listening on ws://0.0.0.0:{port}");

    // Per-IP cap so a single client can't open a swarm of shard WS
    // connections to exhaust the per-thread resource budget. Games with
    // many tabs/devices per household still get 64 concurrent shards.
    let ip_counter = Arc::new(IpConnCounter::default());

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let ip = match stream.peer_addr() {
            Ok(addr) => addr.ip(),
            Err(_) => continue,
        };
        let guard = match ip_counter.acquire(ip) {
            Some(g) => g,
            None => continue,
        };
        let registry = Arc::clone(&registry);
        let sessions = Arc::clone(&sessions);
        thread::spawn(move || {
            // Holding `_guard` for the life of this thread (which lives for
            // the full connection) is what ties the IP slot to the socket.
            let _guard = guard;
            if let Err(e) = handle_connection(stream, registry, sessions) {
                tracing::warn!("[shard-ws] connection error: {e}");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Per-connection handler
// ---------------------------------------------------------------------------

fn handle_connection(
    stream: TcpStream,
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
) -> Result<(), String> {
    // Capture the HTTP handshake so we can read the Request-URI and headers.
    let params = std::sync::Arc::new(Mutex::new(HandshakeParams::default()));
    let params_clone = Arc::clone(&params);

    use tungstenite::handshake::server::{ErrorResponse, Response};
    let ws = accept_hdr(
        stream,
        |req: &Request, mut resp: Response| -> Result<Response, ErrorResponse> {
            let uri = req.uri().to_string();
            let mut p = params_clone.lock().unwrap();
            p.uri = uri;
            let mut selected_protocol: Option<String> = None;
            for (name, value) in req.headers() {
                let lower = name.as_str().to_ascii_lowercase();
                if lower == "authorization" {
                    if let Ok(v) = value.to_str() {
                        p.auth_header = Some(v.to_string());
                    }
                } else if lower == "sec-websocket-protocol" {
                    // Accept a `bearer.<url-encoded-token>` subprotocol as an
                    // alternative to the Authorization header. Browsers can't
                    // set WebSocket headers directly, so this is how a web
                    // client carries a bearer token without putting it in the
                    // URL. Pick the first token that matches our prefix; echo
                    // the exact chosen subprotocol back in the handshake
                    // response, per RFC 6455 §11.3.4 (otherwise some browsers
                    // refuse the connection).
                    if let Ok(v) = value.to_str() {
                        for proto in v.split(',').map(str::trim) {
                            if let Some(encoded) = proto.strip_prefix("bearer.") {
                                if let Ok(decoded) = urldecode_strict(encoded) {
                                    p.bearer_from_subprotocol = Some(decoded);
                                    selected_protocol = Some(proto.to_string());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            if let Some(chosen) = selected_protocol {
                if let Ok(hv) = tungstenite::http::HeaderValue::from_str(&chosen) {
                    resp.headers_mut().insert("Sec-WebSocket-Protocol", hv);
                }
            }
            Ok(resp)
        },
    )
    .map_err(|e| format!("handshake: {e}"))?;

    let params = params.lock().unwrap().clone();
    let query = params
        .uri
        .split_once('?')
        .map(|(_, q)| q.to_string())
        .unwrap_or_default();

    let shard_id = query_param(&query, "shard").ok_or("missing ?shard= parameter")?;
    let sid = query_param(&query, "sid").unwrap_or_else(|| "anon".to_string());

    // Resolve auth token. Preference order:
    //   1. Authorization: Bearer ...   (native clients)
    //   2. Sec-WebSocket-Protocol: bearer.<token>   (browsers)
    //
    // The legacy `?token=` query-string path was removed: it leaked the
    // bearer token into proxy access logs, Referer headers, and browser
    // history. All supported clients can send the subprotocol or header.
    let token = params
        .auth_header
        .as_deref()
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|t| t.to_string())
        .or_else(|| params.bearer_from_subprotocol.clone());
    let auth_ctx = sessions.resolve(token.as_deref());
    let shard_auth = ShardAuth {
        user_id: auth_ctx.user_id.clone(),
        is_admin: auth_ctx.is_admin,
    };

    let shard = registry
        .get(&shard_id)
        .ok_or_else(|| format!("shard \"{shard_id}\" not found"))?;

    let ws = Arc::new(Mutex::new(ws));
    let subscriber_id = SubscriberId::new(sid.clone());

    // Build the sink: every snapshot broadcast becomes a WS binary frame.
    let ws_for_sink = Arc::clone(&ws);
    let sink: pylon_realtime::SnapshotSink = Box::new(move |tick, bytes| {
        let mut payload = Vec::with_capacity(8 + bytes.len() + 2);
        payload.extend_from_slice(&tick.to_be_bytes());
        payload.extend_from_slice(bytes);
        if let Ok(mut s) = ws_for_sink.lock() {
            let _ = s.send(Message::Binary(payload.into()));
        }
    });

    // Register the subscriber, respecting auth.
    match shard.add_subscriber(subscriber_id.clone(), sink, &shard_auth) {
        Ok(()) => {}
        Err(ShardError::Unauthorized(reason)) => {
            let _ = ws
                .lock()
                .unwrap()
                .close(Some(tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Policy,
                    reason: format!("unauthorized: {reason}").into(),
                }));
            return Ok(());
        }
        Err(e) => {
            let _ = ws
                .lock()
                .unwrap()
                .close(Some(tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Again,
                    reason: e.to_string().into(),
                }));
            return Ok(());
        }
    }

    // Read loop — inbound messages from the client become shard inputs.
    // Each message is JSON: {"input": ..., "client_seq"?: N}
    let read_result = loop {
        let msg = {
            let mut s = match ws.lock() {
                Ok(s) => s,
                Err(_) => break Err("ws lock poisoned".to_string()),
            };
            match s.read() {
                Ok(m) => m,
                Err(tungstenite::Error::ConnectionClosed) => break Ok(()),
                Err(tungstenite::Error::AlreadyClosed) => break Ok(()),
                Err(e) => break Err(format!("ws read: {e}")),
            }
        };

        match msg {
            Message::Text(text) => {
                process_input(&shard, &subscriber_id, &shard_auth, text.as_str());
            }
            Message::Binary(bytes) => {
                let text = String::from_utf8_lossy(&bytes).to_string();
                process_input(&shard, &subscriber_id, &shard_auth, &text);
            }
            Message::Ping(payload) => {
                let _ = ws.lock().unwrap().send(Message::Pong(payload));
            }
            Message::Close(_) => break Ok(()),
            _ => {}
        }
    };

    // Clean up.
    shard.remove_subscriber(&subscriber_id);
    if let Err(e) = read_result {
        Err(e)
    } else {
        Ok(())
    }
}

fn process_input(
    shard: &Arc<dyn pylon_realtime::DynShard>,
    subscriber_id: &SubscriberId,
    shard_auth: &ShardAuth,
    text: &str,
) {
    // Envelope shape: { input, client_seq? }
    let envelope: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };
    let input = envelope
        .get("input")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let client_seq = envelope.get("client_seq").and_then(|v| v.as_u64());
    let input_str = serde_json::to_string(&input).unwrap_or_else(|_| "null".into());

    let _ = shard.push_input_json(subscriber_id.clone(), &input_str, client_seq, shard_auth);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct HandshakeParams {
    uri: String,
    auth_header: Option<String>,
    bearer_from_subprotocol: Option<String>,
}

/// Strict percent-decode: fails on malformed input. Used for the WS
/// subprotocol bearer token so we don't silently accept garbage.
fn urldecode_strict(s: &str) -> Result<String, String> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err("truncated percent-encoding".into());
            }
            let hi = (bytes[i + 1] as char)
                .to_digit(16)
                .ok_or("bad hex in percent-encoding")?;
            let lo = (bytes[i + 2] as char)
                .to_digit(16)
                .ok_or("bad hex in percent-encoding")?;
            out.push(((hi << 4) | lo) as u8);
            i += 3;
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "percent-encoded token is not valid UTF-8".into())
}

fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        let k = it.next()?;
        let v = it.next().unwrap_or("");
        if k == key {
            return Some(url_decode(v));
        }
    }
    None
}

fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let Ok(h) =
                    u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
                {
                    out.push(h as char);
                    i += 3;
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }
    out
}

// Silence timeout warnings when clients hold connections open briefly.
#[allow(dead_code)]
fn apply_read_timeout(stream: &TcpStream, dur: Duration) {
    let _ = stream.set_read_timeout(Some(dur));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_param_parses_basic() {
        assert_eq!(
            query_param("shard=match1&sid=p1", "shard"),
            Some("match1".to_string())
        );
        assert_eq!(
            query_param("shard=match1&sid=p1", "sid"),
            Some("p1".to_string())
        );
        assert_eq!(query_param("shard=match1", "missing"), None);
    }

    #[test]
    fn query_param_url_decodes() {
        assert_eq!(
            query_param("name=hello%20world", "name"),
            Some("hello world".to_string())
        );
    }
}
