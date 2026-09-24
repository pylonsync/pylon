//! Bidirectional WebSocket server for real-time shards.
//!
//! Runs on its own port (typically `pylon_port + 3`). Each connection:
//!
//! 1. Parses the request path for `?shard=<id>&sid=<subscriber>`.
//! 2. Looks up the shard in the [`DynShardRegistry`].
//! 3. Runs the subscribe authorization hook, and gets the subscriber's
//!    outbound queue from the shard.
//! 4. Runs two tasks: a writer that drains the queue into the socket, and a
//!    reader that turns client frames into shard inputs.
//! 5. Removes the subscriber when either side ends.
//!
//! The tick thread only pushes into the queue, so no client can stall the
//! shard: an idle client (the reader waits, the writer does not) or a slow
//! one (its queue drops old snapshots, then closes) affects only itself.
//!
//! Connections run as tasks on a small tokio runtime, so an idle connection
//! costs memory, not a thread. Accept stays on one blocking thread with
//! [`crate::accept_tcp`], which avoids the macOS dual-stack accept panic.

use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use pylon_auth::SessionStore;
use pylon_realtime::{DynShardRegistry, ShardAuth, ShardError, SubscriberId};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::{
    handshake::server::{ErrorResponse, Request, Response},
    protocol::{frame::coding::CloseCode, CloseFrame},
    Message,
};

use crate::ip_limit::IpConnCounter;

/// The server sends a ping this often.
const PING_INTERVAL: Duration = Duration::from_secs(20);
/// A connection that sends nothing (not even a pong) for this long is
/// closed. Covers clients that vanished without closing the socket.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

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
    // Dual-stack v6+v4 bind — without this, macOS clients that
    // resolve `localhost` to `::1` (IPv6) hit ECONNREFUSED on what
    // looks like a working server. See crate::bind_dual_stack_tcp.
    let listener = match crate::bind_dual_stack_tcp(port) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[shard-ws] failed to bind port {port}: {e}");
            return;
        }
    };
    tracing::warn!("[shard-ws] listening on ws://[::]:{port} (dual-stack)");
    serve(
        listener,
        registry,
        sessions,
        max_connections_per_ip_from_env(),
    );
}

/// `PYLON_SHARD_WS_MAX_PER_IP`: concurrent shard connections one IP may
/// hold (default 64; 0 = no cap). Raise it when players reach the server
/// through a proxy or a shared NAT that presents one address.
fn max_connections_per_ip_from_env() -> u32 {
    std::env::var("PYLON_SHARD_WS_MAX_PER_IP")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(crate::ip_limit::DEFAULT_MAX_CONNECTIONS_PER_IP)
}

/// Worker threads for the connection runtime: the machine's cores, between
/// 2 and 8. Connections mostly wait; the shard tick threads do the work.
fn worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(2, 8)
}

/// Accept shard connections on `listener` until the process exits.
/// `max_per_ip` caps concurrent connections from one IP (0 = no cap).
pub fn serve(
    listener: TcpListener,
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
    max_per_ip: u32,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads())
        .thread_name("shard-ws")
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::warn!("[shard-ws] could not start the connection runtime: {e}");
            return;
        }
    };

    // Per-IP cap so a single client can't open a swarm of shard WS
    // connections.
    let ip_counter = Arc::new(IpConnCounter::new(if max_per_ip == 0 {
        u32::MAX
    } else {
        max_per_ip
    }));

    loop {
        // Panic-proof accept: libstd's accept/peer_addr assert (panic) on a
        // truncated macOS dual-stack sockaddr. See crate::accept_tcp.
        let (stream, peer_ip) = match crate::accept_tcp(&listener) {
            Ok(v) => v,
            Err(_) => {
                // Transient accept error — keep serving. A 1ms nap avoids a
                // hot spin if the error is sticky (e.g. fd pressure).
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
        };
        // An unparseable peer (the truncation case) buckets under the
        // unspecified address so the per-IP cap still bounds it.
        let ip = peer_ip.unwrap_or(std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED));
        let guard = match ip_counter.acquire(ip) {
            Some(g) => g,
            None => continue,
        };
        if let Err(e) = stream.set_nonblocking(true) {
            tracing::warn!("[shard-ws] could not make the socket non-blocking: {e}");
            continue;
        }
        let _ = stream.set_nodelay(true);
        let registry = Arc::clone(&registry);
        let sessions = Arc::clone(&sessions);
        runtime.spawn(async move {
            // The guard lives as long as the task, which lives for the full
            // connection: that ties the IP slot to the socket.
            let _guard = guard;
            let stream = match tokio::net::TcpStream::from_std(stream) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("[shard-ws] could not register the socket: {e}");
                    return;
                }
            };
            if let Err(e) = handle_connection(stream, registry, sessions).await {
                tracing::warn!("[shard-ws] connection error: {e}");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Per-connection handler
// ---------------------------------------------------------------------------

async fn handle_connection(
    stream: tokio::net::TcpStream,
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
) -> Result<(), String> {
    // Capture the HTTP handshake so we can read the Request-URI and headers.
    let params = Arc::new(Mutex::new(HandshakeParams::default()));
    let params_clone = Arc::clone(&params);

    let ws = tokio_tungstenite::accept_hdr_async(
        stream,
        move |req: &Request, mut resp: Response| -> Result<Response, ErrorResponse> {
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
                if let Ok(hv) = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&chosen)
                {
                    resp.headers_mut().insert("Sec-WebSocket-Protocol", hv);
                }
            }
            Ok(resp)
        },
    )
    .await
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

    let (mut sink, mut source) = ws.split();

    let shard = match registry.get(&shard_id) {
        Some(s) => s,
        None => {
            close_with(
                &mut sink,
                CloseCode::Policy,
                format!("shard \"{shard_id}\" not found"),
            )
            .await;
            return Ok(());
        }
    };

    let subscriber_id = SubscriberId::new(sid);
    let queue = match shard.add_queued_subscriber(subscriber_id.clone(), &shard_auth) {
        Ok(q) => q,
        Err(ShardError::Unauthorized(reason)) => {
            close_with(
                &mut sink,
                CloseCode::Policy,
                format!("unauthorized: {reason}"),
            )
            .await;
            return Ok(());
        }
        Err(e) => {
            close_with(&mut sink, CloseCode::Again, e.to_string()).await;
            return Ok(());
        }
    };

    // Writer: drain the queue into the socket. The tick thread wakes it
    // through the notifier; `Notify` keeps a wakeup that arrives while the
    // writer is busy, so none is lost.
    let wake = Arc::new(Notify::new());
    {
        let wake = Arc::clone(&wake);
        queue.set_notifier(move || wake.notify_one());
    }
    let writer_queue = Arc::clone(&queue);
    let mut writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.tick().await; // the first tick fires at once
        loop {
            while let Some(frame) = writer_queue.pop() {
                let mut payload = Vec::with_capacity(8 + frame.bytes.len());
                payload.extend_from_slice(&frame.tick.to_be_bytes());
                payload.extend_from_slice(&frame.bytes);
                if sink.send(Message::Binary(payload)).await.is_err() {
                    return;
                }
            }
            if writer_queue.is_closed() {
                close_with(&mut sink, CloseCode::Again, "client too slow".into()).await;
                return;
            }
            tokio::select! {
                _ = wake.notified() => {}
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Vec::new())).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    // Reader: inbound messages become shard inputs. Each message is JSON:
    // {"input": ..., "client_seq"?: N}. Any inbound frame, including the
    // pongs to the writer's pings, counts as activity for the idle timeout.
    let mut check = tokio::time::interval(Duration::from_millis(500));
    let mut last_activity = tokio::time::Instant::now();
    let read_result = loop {
        let next = tokio::select! {
            // The writer ended (socket error, or it saw the queue close).
            _ = &mut writer => break Ok(()),
            // The queue closed while the writer is stuck in a send to a
            // client that stopped reading; or the client went silent.
            _ = check.tick() => {
                if queue.is_closed() {
                    break Err("client too slow; outbound queue closed".to_string());
                }
                if last_activity.elapsed() >= IDLE_TIMEOUT {
                    break Err("idle timeout".to_string());
                }
                continue;
            }
            next = source.next() => next,
        };
        last_activity = tokio::time::Instant::now();
        let msg = match next {
            None => break Ok(()),
            Some(Err(e)) => break Err(format!("ws read: {e}")),
            Some(Ok(m)) => m,
        };
        match msg {
            Message::Text(text) => {
                process_input(&shard, &subscriber_id, &shard_auth, text.as_str());
            }
            Message::Binary(bytes) => {
                let text = String::from_utf8_lossy(&bytes).to_string();
                process_input(&shard, &subscriber_id, &shard_auth, &text);
            }
            Message::Close(_) => break Ok(()),
            // tungstenite answers pings itself; pongs only prove liveness.
            _ => {}
        }
    };

    // Removing the subscriber closes its queue. A writer that is still in a
    // send (to a client that stopped reading) is aborted; the socket closes
    // when both halves drop.
    shard.remove_subscriber(&subscriber_id);
    queue.close();
    writer.abort();
    read_result
}

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    Message,
>;

async fn close_with(sink: &mut WsSink, code: CloseCode, reason: String) {
    let _ = sink
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
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
