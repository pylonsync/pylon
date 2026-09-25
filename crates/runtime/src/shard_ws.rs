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
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use pylon_auth::SessionStore;
use pylon_realtime::{
    wire, DynShardRegistry, FrameKind, OutboundQueue, ShardAuth, ShardError, SnapshotFormat,
    SubscriberId,
};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::{
    handshake::server::{ErrorResponse, Request, Response},
    protocol::{frame::coding::CloseCode, CloseFrame},
    Message,
};

use crate::ip_limit::IpConnCounter;
pub use crate::ip_limit::IpConnGuard;

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

/// The runtime every shard connection runs on, whichever port it came in
/// on. Built on first use.
fn runtime() -> Option<&'static tokio::runtime::Runtime> {
    static RT: std::sync::OnceLock<Option<tokio::runtime::Runtime>> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(worker_threads())
            .thread_name("shard-ws")
            .enable_all()
            .build()
            .map_err(|e| tracing::warn!("[shard-ws] could not start the connection runtime: {e}"))
            .ok()
    })
    .as_ref()
}

fn ip_counter(max_per_ip: u32) -> Arc<IpConnCounter> {
    Arc::new(IpConnCounter::new(if max_per_ip == 0 {
        u32::MAX
    } else {
        max_per_ip
    }))
}

/// The per-IP counter for shard connections on the main HTTP port
/// (`PYLON_SHARD_WS_MAX_PER_IP`).
fn main_port_ip_counter() -> &'static Arc<IpConnCounter> {
    static C: std::sync::OnceLock<Arc<IpConnCounter>> = std::sync::OnceLock::new();
    C.get_or_init(|| ip_counter(max_connections_per_ip_from_env()))
}

/// Reserve a main-port shard connection slot for `ip`. `None` when the IP
/// is at its cap; the caller answers 429 before upgrading.
pub fn admit_main_port(ip: std::net::IpAddr) -> Option<IpConnGuard> {
    main_port_ip_counter().acquire(ip)
}

/// Run a shard connection that arrived on the main HTTP port at `/shard`.
/// The server has already sent the 101 and detached `stream` from its HTTP
/// stack; `uri` and `headers` are the upgrade request's.
pub fn serve_upgraded(
    stream: std::net::TcpStream,
    guard: IpConnGuard,
    uri: String,
    headers: Vec<(String, String)>,
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
) {
    let Some(rt) = runtime() else { return };
    if let Err(e) = stream.set_nonblocking(true) {
        tracing::warn!("[shard-ws] could not make the socket non-blocking: {e}");
        return;
    }
    let _ = stream.set_nodelay(true);
    rt.spawn(async move {
        let _guard = guard;
        let stream = match tokio::net::TcpStream::from_std(stream) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("[shard-ws] could not register the socket: {e}");
                return;
            }
        };
        let (params, _) =
            read_handshake(uri, headers.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let started = Instant::now();
        let ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
            stream,
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let end = run_connection(ws, params, registry, sessions).await;
        log_connection_end(started, &end);
    });
}

/// Splice a client connection to the machine that runs its shard (see
/// `shard_route`): `early` is what the machine sent after its 101, which
/// goes to the client first; then bytes flow both ways until either side
/// closes.
pub fn splice(
    client: std::net::TcpStream,
    upstream: std::net::TcpStream,
    early: Vec<u8>,
    slot: Option<Box<dyn Send>>,
) {
    let Some(rt) = runtime() else { return };
    for s in [&client, &upstream] {
        if let Err(e) = s.set_nonblocking(true) {
            tracing::warn!("[shard-ws] could not make the socket non-blocking: {e}");
            return;
        }
        let _ = s.set_nodelay(true);
    }
    rt.spawn(async move {
        use tokio::io::AsyncWriteExt;
        let _slot = slot;
        let (mut client, mut upstream) = match (
            tokio::net::TcpStream::from_std(client),
            tokio::net::TcpStream::from_std(upstream),
        ) {
            (Ok(c), Ok(u)) => (c, u),
            _ => {
                tracing::warn!("[shard-ws] could not register the proxied sockets");
                return;
            }
        };
        if !early.is_empty() && client.write_all(&early).await.is_err() {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    });
}

/// The subprotocol to echo in a shard upgrade response, when the request
/// offered a `bearer.` or `ticket.` one.
pub fn chosen_subprotocol<'a>(headers: impl Iterator<Item = (&'a str, &'a str)>) -> Option<String> {
    read_handshake(String::new(), headers).1
}

/// Accept shard connections on `listener` until the process exits.
/// `max_per_ip` caps concurrent connections from one IP (0 = no cap).
pub fn serve(
    listener: TcpListener,
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
    max_per_ip: u32,
) {
    let Some(runtime) = runtime() else { return };
    // Per-IP cap so a single client can't open a swarm of shard WS
    // connections.
    let ip_counter = ip_counter(max_per_ip);

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
            // A shard another machine runs: pass the connection there. That
            // machine logs the connection's end.
            let stream = match route_to_owner(stream, &registry, ip).await {
                Some(stream) => stream,
                None => return,
            };
            let started = Instant::now();
            let end = handle_connection(stream, registry, sessions).await;
            log_connection_end(started, &end);
        });
    }
}

/// The upgrade request's head, read without consuming it, or None when it
/// does not arrive complete within a few seconds.
async fn peek_head(stream: &tokio::net::TcpStream) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 16 * 1024];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let n = tokio::time::timeout_at(deadline, stream.peek(&mut buf))
            .await
            .ok()?
            .ok()?;
        if let Some(i) = buf[..n].windows(4).position(|w| w == b"\r\n\r\n") {
            return Some(buf[..i + 4].to_vec());
        }
        if n == buf.len() || tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// For a connection to a shard another machine runs (see `shard_cluster`),
/// proxy it to that machine's `/shard` and return None; otherwise hand the
/// stream back, unread, for this machine to serve.
async fn route_to_owner(
    stream: tokio::net::TcpStream,
    registry: &Arc<dyn DynShardRegistry>,
    client_ip: std::net::IpAddr,
) -> Option<tokio::net::TcpStream> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Some(head) = peek_head(&stream).await else {
        return Some(stream);
    };
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let url = lines.next()?.split_whitespace().nth(1)?.to_string();
    let Some(shard) = crate::shard_route::shard_of(&url) else {
        return Some(stream);
    };
    // The lookup reads the shard directory with a blocking Postgres client,
    // which must not run on a tokio worker.
    let lookup = Arc::clone(registry);
    let location = tokio::task::spawn_blocking(move || lookup.locate(&shard))
        .await
        .ok()?;
    let pylon_realtime::ShardLocation::Remote {
        machine_id,
        address: Some(address),
        ..
    } = location
    else {
        return Some(stream);
    };
    // The owner serves shard WebSockets at /shard on its main port.
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let target_url = format!("/shard?{query}");
    let host = crate::shard_route::authority(&address)?.to_string();
    let forwarded = crate::shard_route::forwarded_value(
        &client_ip.to_string(),
        "GET",
        &target_url,
        &machine_id,
    )?;
    let mut out = format!("GET {target_url} HTTP/1.1\r\nHost: {host}\r\n");
    for line in lines.filter(|l| !l.is_empty()) {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let k = k.trim();
        if k.eq_ignore_ascii_case("host")
            || k.eq_ignore_ascii_case(crate::shard_cluster::FORWARDED_HEADER)
        {
            continue;
        }
        out.push_str(&format!("{k}: {}\r\n", v.trim()));
    }
    out.push_str(&format!(
        "{}: {forwarded}\r\n\r\n",
        crate::shard_cluster::FORWARDED_HEADER
    ));
    let mut client = stream;
    let mut consumed = vec![0u8; head.len()];
    if client.read_exact(&mut consumed).await.is_err() {
        return None;
    }
    let mut upstream = match tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect(host.as_str()),
    )
    .await
    {
        Ok(Ok(s)) => s,
        _ => {
            tracing::warn!("[shard-ws] could not reach machine {machine_id} at {address}");
            let _ = client
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            return None;
        }
    };
    let _ = upstream.set_nodelay(true);
    if upstream.write_all(out.as_bytes()).await.is_err() {
        return None;
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    None
}

// ---------------------------------------------------------------------------
// Per-connection handler
// ---------------------------------------------------------------------------

async fn handle_connection(
    stream: tokio::net::TcpStream,
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
) -> ConnectionEnd {
    // Capture the HTTP handshake so we can read the Request-URI and headers.
    let params = Arc::new(Mutex::new(HandshakeParams::default()));
    let params_clone = Arc::clone(&params);

    let ws = tokio_tungstenite::accept_hdr_async(
        stream,
        move |req: &Request, mut resp: Response| -> Result<Response, ErrorResponse> {
            let headers = req
                .headers()
                .iter()
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str(), v)));
            let (p, selected_protocol) = read_handshake(req.uri().to_string(), headers);
            *params_clone.lock().unwrap() = p;
            if let Some(chosen) = selected_protocol {
                if let Ok(hv) = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&chosen)
                {
                    resp.headers_mut().insert("Sec-WebSocket-Protocol", hv);
                }
            }
            Ok(resp)
        },
    )
    .await;
    let ws = match ws {
        Ok(ws) => ws,
        Err(e) => return ConnectionEnd::Closed(format!("handshake: {e}")),
    };

    let params = params.lock().unwrap().clone();
    run_connection(ws, params, registry, sessions).await
}

/// How a shard connection ended. Written to the request log when the
/// connection closes.
enum ConnectionEnd {
    /// The client closed the connection.
    ClientClosed,
    /// The server closed the connection, or the connection failed. The text
    /// is the reason.
    Closed(String),
}

/// Write the end of a shard connection to the request log, with its
/// duration. The upgrade itself is logged as a `GET /shard 101`, which shows
/// nothing about a connection that the server rejects right after the
/// upgrade. This row shows the reason.
fn log_connection_end(started: Instant, end: &ConnectionEnd) {
    let ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    let reason = match end {
        ConnectionEnd::ClientClosed => None,
        ConnectionEnd::Closed(reason) => {
            tracing::info!("[shard-ws] connection closed after {ms}ms: {reason}");
            Some(reason.as_str())
        }
    };
    crate::metrics::record_log_row("WS", "/shard", 101, ms, 0, 0, reason);
}

/// Read the shard parameters from an upgrade request's URI and headers.
/// Returns them and the subprotocol to echo back, if any.
fn read_handshake<'a>(
    uri: String,
    headers: impl Iterator<Item = (&'a str, &'a str)>,
) -> (HandshakeParams, Option<String>) {
    let mut p = HandshakeParams {
        uri,
        ..Default::default()
    };
    let mut selected_protocol: Option<String> = None;
    for (name, v) in headers {
        let lower = name.to_ascii_lowercase();
        if lower == "authorization" {
            p.auth_header = Some(v.to_string());
        } else if lower == "x-pylon-shard-ticket" {
            p.ticket = Some(v.to_string());
        } else if lower == "sec-websocket-protocol" {
            // Accept a `bearer.<url-encoded-token>` subprotocol as an
            // alternative to the Authorization header. Browsers can't set
            // WebSocket headers directly, so this is how a web client carries
            // a bearer token without putting it in the URL. The exact chosen
            // subprotocol is echoed back in the handshake response, per RFC
            // 6455 §11.3.4 (otherwise some browsers refuse the connection).
            // A `ticket.<url-encoded-ticket>` subprotocol carries a shard
            // ticket the same way. Only one subprotocol can be selected in
            // the response; the bearer one wins.
            for proto in v.split(',').map(str::trim) {
                if let Some(encoded) = proto.strip_prefix("bearer.") {
                    if p.bearer_from_subprotocol.is_none() {
                        if let Ok(decoded) = urldecode_strict(encoded) {
                            p.bearer_from_subprotocol = Some(decoded);
                            selected_protocol = Some(proto.to_string());
                        }
                    }
                } else if let Some(encoded) = proto.strip_prefix("ticket.") {
                    if let Ok(decoded) = urldecode_strict(encoded) {
                        p.ticket = Some(decoded);
                        selected_protocol.get_or_insert_with(|| proto.to_string());
                    }
                }
            }
        }
    }
    (p, selected_protocol)
}

/// One shard connection after the WebSocket handshake.
async fn run_connection(
    ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    params: HandshakeParams,
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
) -> ConnectionEnd {
    let query = params
        .uri
        .split_once('?')
        .map(|(_, q)| q.to_string())
        .unwrap_or_default();

    let Some(shard_id) = query_param(&query, "shard") else {
        return ConnectionEnd::Closed("missing ?shard= parameter".into());
    };
    let sid = query_param(&query, "sid").unwrap_or_else(|| "anon".to_string());
    // Wire protocol version (see pylon_realtime::wire): `?v=2`, else 1.
    let version: u8 = if query_param(&query, "v").as_deref() == Some("2") {
        2
    } else {
        1
    };

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
    let (mut sink, mut source) = ws.split();
    let shard_auth: ShardAuth =
        match crate::shard_tickets::shard_auth(&auth_ctx, params.ticket.as_deref()) {
            Ok(a) => a,
            Err(e) => {
                let reason = format!("unauthorized: {e}");
                close_with(&mut sink, CloseCode::Policy, reason.clone()).await;
                return ConnectionEnd::Closed(reason);
            }
        };

    let shard = match registry.get(&shard_id) {
        Some(s) => s,
        None => {
            let reason = format!("shard \"{shard_id}\" not found");
            close_with(&mut sink, CloseCode::Policy, reason.clone()).await;
            return ConnectionEnd::Closed(reason);
        }
    };

    let subscriber_id = SubscriberId::new(sid);
    // The authorize hook runs under the shard's state lock and may run
    // module code (a WebAssembly shard), so keep it off the async workers.
    let joined = {
        let (shard, sid, auth) = (
            Arc::clone(&shard),
            subscriber_id.clone(),
            shard_auth.clone(),
        );
        tokio::task::spawn_blocking(move || shard.add_queued_subscriber(sid, &auth)).await
    };
    let joined = match joined {
        Ok(r) => r,
        Err(e) => Err(ShardError::Other(format!("subscribe task failed: {e}"))),
    };
    let queue = match joined {
        Ok(q) => q,
        Err(ShardError::Unauthorized(reason)) => {
            let reason = format!("unauthorized: {reason}");
            close_with(&mut sink, CloseCode::Policy, reason.clone()).await;
            return ConnectionEnd::Closed(reason);
        }
        Err(e) => {
            let reason = e.to_string();
            close_with(&mut sink, CloseCode::Again, reason.clone()).await;
            return ConnectionEnd::Closed(reason);
        }
    };

    // Writer: drain the queue into the socket. The tick thread wakes it
    // through the notifier; `Notify` keeps a wakeup that arrives while the
    // writer is busy, so none is lost. It returns the reason it stopped.
    let wake = Arc::new(Notify::new());
    {
        let wake = Arc::clone(&wake);
        queue.set_notifier(move || wake.notify_one());
    }
    let writer_queue = Arc::clone(&queue);
    let writer_shard = Arc::clone(&shard);
    let codec = wire::codec_byte(shard.snapshot_format());
    let mut writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.tick().await; // the first tick fires at once
        loop {
            while let Some(frame) = writer_queue.pop() {
                let payload = match (version, frame.kind) {
                    (2, FrameKind::Snapshot) => wire::frame_v2(
                        wire::kind::SNAPSHOT,
                        codec,
                        frame.tick,
                        frame.ack,
                        &frame.bytes,
                    ),
                    (2, FrameKind::InputRejected) => wire::frame_v2(
                        wire::kind::INPUT_REJECTED,
                        codec,
                        frame.tick,
                        frame.ack,
                        &frame.bytes,
                    ),
                    (2, FrameKind::Replication) => wire::frame_v2(
                        wire::kind::REPLICATION,
                        wire::codec::REPLICATION,
                        frame.tick,
                        frame.ack,
                        &frame.bytes,
                    ),
                    (_, FrameKind::Snapshot) => wire::frame_v1(frame.tick, &frame.bytes),
                    // Version 1 has no rejection frame.
                    (_, FrameKind::InputRejected) => continue,
                    // Version 1 cannot mark a frame as replication.
                    (_, FrameKind::Replication) => {
                        let reason =
                            "this shard replicates entities; connect with protocol v=2".to_string();
                        close_with(&mut sink, CloseCode::Protocol, reason.clone()).await;
                        return reason;
                    }
                };
                if let Err(e) = sink.send(Message::Binary(payload)).await {
                    return format!("ws write: {e}");
                }
            }
            if writer_queue.is_closed() {
                let (code, reason) = if writer_shard.is_running() {
                    (CloseCode::Again, "client too slow")
                } else {
                    (CloseCode::Normal, "shard stopped")
                };
                close_with(&mut sink, code, reason.into()).await;
                return reason.to_string();
            }
            tokio::select! {
                _ = wake.notified() => {}
                _ = ping.tick() => {
                    if let Err(e) = sink.send(Message::Ping(Vec::new())).await {
                        return format!("ws write: {e}");
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
    let end = loop {
        let next = tokio::select! {
            // The writer ended (socket error, or it saw the queue close).
            stopped = &mut writer => {
                break ConnectionEnd::Closed(
                    stopped.unwrap_or_else(|e| format!("writer task failed: {e}")),
                );
            }
            // The queue closed while the writer is stuck in a send to a
            // client that stopped reading; or the client went silent.
            _ = check.tick() => {
                if queue.is_closed() {
                    break ConnectionEnd::Closed("client too slow; outbound queue closed".into());
                }
                if last_activity.elapsed() >= IDLE_TIMEOUT {
                    break ConnectionEnd::Closed("idle timeout".into());
                }
                continue;
            }
            next = source.next() => next,
        };
        last_activity = tokio::time::Instant::now();
        let msg = match next {
            None => break ConnectionEnd::ClientClosed,
            Some(Err(e)) => break ConnectionEnd::Closed(format!("ws read: {e}")),
            Some(Ok(m)) => m,
        };
        match msg {
            // Text frames are JSON in every version.
            Message::Text(text) => {
                process_input(
                    &shard,
                    &queue,
                    &subscriber_id,
                    &shard_auth,
                    SnapshotFormat::Json,
                    text.as_bytes().to_vec(),
                    version,
                )
                .await;
            }
            // Binary frames are in the shard's codec in version 2, JSON in
            // version 1.
            Message::Binary(bytes) => {
                let format = if version == 2 {
                    shard.snapshot_format()
                } else {
                    SnapshotFormat::Json
                };
                process_input(
                    &shard,
                    &queue,
                    &subscriber_id,
                    &shard_auth,
                    format,
                    bytes.to_vec(),
                    version,
                )
                .await;
            }
            Message::Close(_) => break ConnectionEnd::ClientClosed,
            // tungstenite answers pings itself; pongs only prove liveness.
            _ => {}
        }
    };

    // Removing this connection's subscription closes its queue. A writer that is still in a
    // send (to a client that stopped reading) is aborted; the socket closes
    // when both halves drop.
    shard.remove_queued_subscriber(&queue);
    writer.abort();
    end
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

/// Queue one input envelope. In version 2 a refused input gets an
/// input-rejected frame on this connection.
///
/// The push runs the authorize hook under the shard's state lock, which a
/// tick holds, and it may run module code: it runs on a blocking thread, so
/// a slow shard does not stall the async workers every connection shares.
/// The reader awaits it, so one connection's inputs stay in order.
async fn process_input(
    shard: &Arc<dyn pylon_realtime::DynShard>,
    queue: &Arc<OutboundQueue>,
    subscriber_id: &SubscriberId,
    shard_auth: &ShardAuth,
    format: SnapshotFormat,
    bytes: Vec<u8>,
    version: u8,
) {
    let pushed = {
        let (shard, sid, auth) = (Arc::clone(shard), subscriber_id.clone(), shard_auth.clone());
        tokio::task::spawn_blocking(move || shard.push_input_envelope(sid, format, &bytes, &auth))
            .await
    };
    let rejection = match pushed {
        Ok(Ok(_)) => return,
        Ok(Err(rejection)) => rejection,
        Err(e) => {
            tracing::warn!("[shard-ws] input task failed: {e}");
            return;
        }
    };
    if version != 2 {
        return;
    }
    match pylon_realtime::encode_snapshot(&rejection, shard.snapshot_format()) {
        Ok(encoded) => {
            queue.push_rejection(
                shard.tick_number(),
                shard.ack(subscriber_id),
                Arc::from(encoded),
            );
        }
        Err(e) => tracing::warn!("[shard-ws] rejection encode failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct HandshakeParams {
    uri: String,
    auth_header: Option<String>,
    bearer_from_subprotocol: Option<String>,
    /// A shard ticket from `X-Pylon-Shard-Ticket` or a `ticket.` subprotocol.
    ticket: Option<String>,
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
