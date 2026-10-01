//! Shard connections over WebTransport (HTTP/3 over QUIC).
//!
//! A replicating shard's updates travel as QUIC datagrams, which a lost
//! packet delays by nothing but itself; over a WebSocket (TCP) one lost
//! packet holds back everything behind it. See `pylon_realtime::wire`
//! (version 3) and `pylon_realtime::replication` for the protocol.
//!
//! A session at `/shard`:
//!
//! 1. The client opens one bidirectional stream and sends its hello: a
//!    4-byte big-endian length, then JSON `{shard, sid, ticket?, token?}`.
//!    Credentials travel here, never in the URL.
//! 2. The server sends frames on that stream: each a 4-byte big-endian
//!    length, then a wire version 2 frame (18-byte header, payload). It
//!    sends the subscription's replication updates as bare QUIC datagrams
//!    (`pylon_replication::datagram`).
//! 3. The client sends messages on the stream (length, then a version 3
//!    message, see `wire::client`) and acks as bare datagrams.
//!
//! **Certificates.** The server makes its own self-signed certificate (no
//! public CA), and the client pins it with `serverCertificateHashes`, which
//! allows a validity of 14 days at most. The server keeps a current and a
//! next certificate, each valid for 13 days, serves the current one, and
//! every 6 days makes the next one current. `GET /_pylon/shard/webtransport`
//! gives the URL and both hashes, so a client that fetched them just before
//! a rotation still gets in.
//!
//! **Configuration.** `PYLON_WEBTRANSPORT_PORT` turns it on (a UDP port).
//! `PYLON_WEBTRANSPORT_BIND` is the address to bind (default: every
//! address; on Fly, `fly-global-services`). `PYLON_WEBTRANSPORT_URL` is what
//! clients connect to (default `https://localhost:<port>/shard`).

use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use pylon_auth::SessionStore;
use pylon_realtime::{
    wire, DynShardRegistry, FrameKind, ShardAuth, ShardError, SnapshotFormat, SubscriberId,
};
use serde::Deserialize;
use wtransport::endpoint::endpoint_side::Server;
use wtransport::error::ConnectionError;
use wtransport::{Connection, Endpoint, Identity, RecvStream, SendStream, ServerConfig, VarInt};

use crate::shard_ws::{frame_v2_of, log_connection_end_as, process_input, ConnectionEnd};

/// How long a certificate is valid. Browsers accept at most 14 days.
const CERT_VALIDITY_DAYS: u32 = 13;
/// How often the next certificate becomes the current one.
const ROTATE_EVERY: Duration = Duration::from_secs(6 * 24 * 3600);
/// A hello larger than this is refused.
const MAX_HELLO: usize = 16 * 1024;
/// A client message on the stream larger than this closes the session.
const MAX_CLIENT_MESSAGE: usize = 1 << 20;
/// The client must open its stream and send its hello within this.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// Smallest datagram a session must carry: a datagram header and an update.
const MIN_DATAGRAM: usize = 128;

/// Session close codes (WebTransport application error codes).
mod close_code {
    pub const NORMAL: u32 = 0;
    /// Refused: bad credentials, an unknown shard.
    pub const POLICY: u32 = 1;
    /// The client broke the protocol.
    pub const PROTOCOL: u32 = 2;
    /// Try again: the client was too slow.
    pub const AGAIN: u32 = 3;
}

/// Where WebTransport listens and what clients connect to.
#[derive(Debug, Clone)]
pub struct WebTransportConfig {
    pub port: u16,
    /// Host to bind; None binds every address (IPv4 and IPv6).
    pub bind: Option<String>,
    /// The URL clients connect to.
    pub url: String,
}

impl WebTransportConfig {
    /// From `PYLON_WEBTRANSPORT_*`; None when WebTransport is off.
    pub fn from_env() -> Option<Self> {
        let port: u16 = std::env::var("PYLON_WEBTRANSPORT_PORT")
            .ok()?
            .parse()
            .ok()?;
        let bind = std::env::var("PYLON_WEBTRANSPORT_BIND")
            .ok()
            .filter(|s| !s.is_empty());
        let url = std::env::var("PYLON_WEBTRANSPORT_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("https://localhost:{port}/shard"));
        Some(Self { port, bind, url })
    }

    /// The certificate's subject names: the URL's host, and localhost.
    fn subject_alt_names(&self) -> Vec<String> {
        let mut names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        let host = self
            .url
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(&self.url)
            .split('/')
            .next()
            .unwrap_or("");
        // Strip a port, and IPv6 brackets.
        let host = match host.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(""),
            None => host.rsplit_once(':').map_or(host, |(h, _)| h),
        };
        if !host.is_empty() && !names.iter().any(|n| n == host) {
            names.push(host.to_string());
        }
        names
    }
}

struct Certs {
    current: Identity,
    next: Identity,
    current_since: Instant,
}

/// The running server: its URL and certificates.
pub struct WebTransport {
    config: WebTransportConfig,
    certs: Mutex<Certs>,
}

static RUNNING: OnceLock<Arc<WebTransport>> = OnceLock::new();

/// The other machines' certificate hashes, from the shard directory (see
/// `set_cluster_hashes`).
static CLUSTER_HASHES: Mutex<Vec<[u8; 32]>> = Mutex::new(Vec::new());

pub fn is_running() -> bool {
    RUNNING.get().is_some()
}

/// This machine's current and next certificate hashes.
fn own_hashes() -> Option<Vec<[u8; 32]>> {
    let wt = RUNNING.get()?;
    let certs = wt.certs.lock().unwrap();
    let hash = |id: &Identity| *id.certificate_chain().as_slice()[0].hash().as_ref();
    Some(vec![hash(&certs.current), hash(&certs.next)])
}

/// This machine's hashes as the shard directory stores them: base64,
/// comma-separated. None when WebTransport does not run here.
pub fn own_hashes_text() -> Option<String> {
    use base64::Engine;
    let hashes = own_hashes()?;
    Some(
        hashes
            .iter()
            .map(|h| base64::engine::general_purpose::STANDARD.encode(h))
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// Record every live machine's hashes (their `own_hashes_text`). On Fly a
/// client's UDP may reach any machine of the app, so a client must accept
/// all their certificates.
pub fn set_cluster_hashes<'a>(texts: impl Iterator<Item = &'a str>) {
    use base64::Engine;
    let mut all: Vec<[u8; 32]> = texts
        .flat_map(|t| t.split(','))
        .filter_map(|h| {
            base64::engine::general_purpose::STANDARD
                .decode(h.trim())
                .ok()
        })
        .filter_map(|b| <[u8; 32]>::try_from(b).ok())
        .collect();
    all.sort_unstable();
    all.dedup();
    *CLUSTER_HASHES.lock().unwrap() = all;
}

/// The URL and certificate hashes clients need, when WebTransport runs
/// here: this machine's, then the other live machines'.
pub fn endpoint_info() -> Option<(String, Vec<[u8; 32]>)> {
    let wt = RUNNING.get()?;
    let mut hashes = own_hashes()?;
    for h in CLUSTER_HASHES.lock().unwrap().iter() {
        if !hashes.contains(h) {
            hashes.push(*h);
        }
    }
    Some((wt.config.url.clone(), hashes))
}

/// The body of `GET /_pylon/shard/webtransport`: `{"url", "certHashes"}`,
/// the hashes base64 (standard, padded), or None when WebTransport is off.
pub fn endpoint_info_json() -> Option<String> {
    use base64::Engine;
    let (url, hashes) = endpoint_info()?;
    let hashes: Vec<String> = hashes
        .iter()
        .map(|h| base64::engine::general_purpose::STANDARD.encode(h))
        .collect();
    Some(serde_json::json!({ "url": url, "certHashes": hashes }).to_string())
}

fn self_signed(config: &WebTransportConfig) -> Result<Identity, String> {
    Identity::self_signed_builder()
        .subject_alt_names(config.subject_alt_names())
        .from_now_utc()
        .validity_days(CERT_VALIDITY_DAYS)
        .build()
        .map_err(|e| format!("self-signed certificate: {e}"))
}

fn server_config(config: &WebTransportConfig, identity: Identity) -> Result<ServerConfig, String> {
    let builder = ServerConfig::builder();
    let builder = match &config.bind {
        None => builder.with_bind_default(config.port),
        Some(host) => {
            let addr = (host.as_str(), config.port)
                .to_socket_addrs()
                .map_err(|e| format!("resolve {host}: {e}"))?
                .next()
                .ok_or_else(|| format!("{host} resolves to no address"))?;
            builder.with_bind_address(addr)
        }
    };
    Ok(builder
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(5)))
        .max_idle_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| format!("idle timeout: {e}"))?
        .build())
}

/// Start serving shard sessions over WebTransport. Returns the bound
/// server, or why it could not start.
pub fn start(
    config: WebTransportConfig,
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
) -> Result<Arc<WebTransport>, String> {
    let rt = crate::shard_ws::runtime().ok_or("no connection runtime")?;
    let current = self_signed(&config)?;
    let next = self_signed(&config)?;
    let server_cfg = server_config(&config, current.clone_identity())?;
    let _guard = rt.enter();
    let endpoint =
        Endpoint::server(server_cfg).map_err(|e| format!("bind UDP port {}: {e}", config.port))?;
    let wt = Arc::new(WebTransport {
        config,
        certs: Mutex::new(Certs {
            current,
            next,
            current_since: Instant::now(),
        }),
    });
    let _ = RUNNING.set(Arc::clone(&wt));
    tracing::warn!(
        "[shard-wt] WebTransport on UDP port {} ({})",
        wt.config.port,
        wt.config.url
    );
    let endpoint = Arc::new(endpoint);
    rt.spawn(rotate(Arc::clone(&wt), Arc::clone(&endpoint)));
    rt.spawn(accept_loop(endpoint, registry, sessions));
    Ok(wt)
}

/// Make the next certificate current every [`ROTATE_EVERY`].
async fn rotate(wt: Arc<WebTransport>, endpoint: Arc<Endpoint<Server>>) {
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        let identity = {
            let mut certs = wt.certs.lock().unwrap();
            if certs.current_since.elapsed() < ROTATE_EVERY {
                continue;
            }
            let fresh = match self_signed(&wt.config) {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!("[shard-wt] certificate rotation: {e}");
                    continue;
                }
            };
            let next = std::mem::replace(&mut certs.next, fresh);
            certs.current = next;
            certs.current_since = Instant::now();
            certs.current.clone_identity()
        };
        match server_config(&wt.config, identity)
            .and_then(|c| endpoint.reload_config(c, false).map_err(|e| e.to_string()))
        {
            Ok(()) => tracing::info!("[shard-wt] rotated the certificate"),
            Err(e) => tracing::warn!("[shard-wt] certificate rotation: {e}"),
        }
    }
}

async fn accept_loop(
    endpoint: Arc<Endpoint<Server>>,
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
) {
    loop {
        let incoming = endpoint.accept().await;
        let (registry, sessions) = (Arc::clone(&registry), Arc::clone(&sessions));
        tokio::spawn(async move {
            let request = match incoming.await {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!("[shard-wt] handshake: {e}");
                    return;
                }
            };
            if request.path().split('?').next() != Some("/shard") {
                request.not_found().await;
                return;
            }
            // The same cap as shard WebSockets, counted together
            // (`PYLON_SHARD_WS_MAX_PER_IP`). The address is the client's
            // own: the QUIC handshake has validated it.
            let ip = request.remote_address().ip().to_canonical();
            let Some(_slot) = crate::shard_ws::admit_main_port(ip) else {
                request.too_many_requests().await;
                return;
            };
            let conn = match request.accept().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!("[shard-wt] accept: {e}");
                    return;
                }
            };
            let started = Instant::now();
            let end = run_session(&conn, registry, sessions).await;
            log_connection_end_as("WT", started, &end);
        });
    }
}

#[derive(Deserialize)]
struct Hello {
    shard: String,
    sid: String,
    #[serde(default)]
    ticket: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

/// Read one length-prefixed message; None at a clean end of the stream.
async fn read_message(recv: &mut RecvStream, max: usize) -> Result<Option<Vec<u8>>, String> {
    let mut len = [0u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(wtransport::error::StreamReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(format!("stream read: {e}")),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > max {
        return Err(format!("a {len}-byte message (at most {max})"));
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| format!("stream read: {e}"))?;
    Ok(Some(buf))
}

/// The client's stream messages, read on their own task. A read that a
/// `select!` drops halfway loses the bytes it already took and leaves the
/// stream in the middle of a message, so a session's loop waits on this
/// channel, which loses nothing, instead of on the stream.
struct StreamReader {
    rx: tokio::sync::mpsc::Receiver<Result<Option<Vec<u8>>, String>>,
    task: tokio::task::JoinHandle<()>,
}

impl StreamReader {
    fn spawn(mut recv: RecvStream) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let task = tokio::spawn(async move {
            loop {
                let message = read_message(&mut recv, MAX_CLIENT_MESSAGE).await;
                let last = !matches!(message, Ok(Some(_)));
                if tx.send(message).await.is_err() || last {
                    return;
                }
            }
        });
        Self { rx, task }
    }

    /// The next message; `Ok(None)` at the end of the stream.
    async fn next(&mut self) -> Result<Option<Vec<u8>>, String> {
        self.rx.recv().await.unwrap_or(Ok(None))
    }
}

impl Drop for StreamReader {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn write_message(send: &mut SendStream, bytes: &[u8]) -> Result<(), String> {
    let len = u32::try_from(bytes.len()).map_err(|_| "a frame over 4 GiB".to_string())?;
    send.write_all(&len.to_be_bytes())
        .await
        .map_err(|e| format!("stream write: {e}"))?;
    send.write_all(bytes)
        .await
        .map_err(|e| format!("stream write: {e}"))
}

/// How a session ended once its connection closed: by the client (it closed
/// the session, or the browser went away), or for a reason worth logging.
fn ended_by(error: ConnectionError) -> ConnectionEnd {
    match error {
        // wtransport closes the connection itself when the client closes
        // the session; the server's own closes do not come here.
        ConnectionError::ApplicationClosed(_)
        | ConnectionError::ConnectionClosed(_)
        | ConnectionError::LocallyClosed => ConnectionEnd::ClientClosed,
        ConnectionError::TimedOut => ConnectionEnd::Closed("the connection timed out".into()),
        other => ConnectionEnd::Closed(other.to_string()),
    }
}

/// A stream read failed: when the whole connection closed, why; else the
/// read error (the client reset the stream).
async fn read_failed(conn: &Connection, error: String) -> ConnectionEnd {
    match tokio::time::timeout(Duration::from_millis(100), conn.closed()).await {
        Ok(closed) => ended_by(closed),
        Err(_) => ConnectionEnd::Closed(error),
    }
}

/// How long a closing notice may take to reach the client, and how long
/// the client then has to close the session itself.
const NOTICE_WAIT: Duration = Duration::from_secs(1);

/// Close the session with `code` and `reason`, telling the client first.
/// wtransport closes the QUIC connection without a WebTransport
/// session-close capsule, so a browser's `closed` never sees the code or
/// the reason. A closing frame on the stream carries them, and the client
/// closes the session when it reads it; the server closes it regardless
/// after [`NOTICE_WAIT`].
async fn close_after_notice(
    conn: &Connection,
    send: &mut SendStream,
    code: u32,
    reason: &str,
) -> ConnectionEnd {
    let body = serde_json::json!({ "code": code, "reason": reason }).to_string();
    let notice = wire::frame_v2(
        wire::kind::CLOSING,
        wire::codec::JSON,
        0,
        0,
        body.as_bytes(),
    );
    // `finish` completes when the client has acknowledged the data.
    let told = async {
        write_message(send, &notice).await.ok()?;
        send.finish().await.ok()
    };
    if let Ok(Some(())) = tokio::time::timeout(NOTICE_WAIT, told).await {
        let _ = tokio::time::timeout(NOTICE_WAIT, conn.closed()).await;
    }
    close(conn, code, reason)
}

fn close(conn: &Connection, code: u32, reason: &str) -> ConnectionEnd {
    // A close reason is at most 1024 bytes in WebTransport.
    let mut end = reason.len().min(1024);
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    conn.close(VarInt::from_u32(code), &reason.as_bytes()[..end]);
    ConnectionEnd::Closed(reason.to_string())
}

async fn run_session(
    conn: &Connection,
    registry: Arc<dyn DynShardRegistry>,
    sessions: Arc<SessionStore>,
) -> ConnectionEnd {
    let opened = tokio::time::timeout(HELLO_TIMEOUT, async {
        let (send, mut recv) = conn
            .accept_bi()
            .await
            .map_err(|e| format!("no stream: {e}"))?;
        let hello = read_message(&mut recv, MAX_HELLO)
            .await?
            .ok_or("the stream ended before the hello")?;
        let hello: Hello =
            serde_json::from_slice(&hello).map_err(|e| format!("a bad hello: {e}"))?;
        Ok::<_, String>((send, recv, hello))
    })
    .await;
    let (mut send, recv, hello) = match opened {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return close(conn, close_code::PROTOCOL, &e),
        Err(_) => return close(conn, close_code::PROTOCOL, "no hello in time"),
    };

    let auth_ctx = sessions.resolve(hello.token.as_deref());
    let shard_auth: ShardAuth =
        match crate::shard_tickets::shard_auth(&auth_ctx, hello.ticket.as_deref()) {
            Ok(a) => a,
            Err(e) => {
                return close_after_notice(
                    conn,
                    &mut send,
                    close_code::POLICY,
                    &format!("unauthorized: {e}"),
                )
                .await
            }
        };
    let datagram_max = conn.max_datagram_size().unwrap_or(0);
    if datagram_max < MIN_DATAGRAM {
        return close_after_notice(
            conn,
            &mut send,
            close_code::PROTOCOL,
            "the connection carries no datagrams",
        )
        .await;
    }
    // The lookup may read the shard directory with a blocking Postgres
    // client, which must not run on an async worker.
    let found = {
        let (registry, id) = (Arc::clone(&registry), hello.shard.clone());
        tokio::task::spawn_blocking(move || match registry.get(&id) {
            Some(shard) => Ok(shard),
            None => Err(registry.locate(&id)),
        })
        .await
    };
    let shard = match found {
        Ok(Ok(shard)) => shard,
        // Another machine runs it: QUIC cannot be handed over, so this
        // machine relays the session there (see `relay`).
        Ok(Err(pylon_realtime::ShardLocation::Remote {
            machine_id,
            address: Some(address),
            ..
        })) => {
            return relay(
                conn,
                send,
                recv,
                &hello,
                datagram_max,
                &machine_id,
                &address,
            )
            .await;
        }
        Ok(Err(_)) => {
            return close_after_notice(
                conn,
                &mut send,
                close_code::POLICY,
                &format!("shard \"{}\" not found", hello.shard),
            )
            .await;
        }
        Err(e) => {
            return close_after_notice(
                conn,
                &mut send,
                close_code::AGAIN,
                &format!("lookup task failed: {e}"),
            )
            .await
        }
    };

    let subscriber_id = SubscriberId::new(hello.sid);
    let joined = {
        let (shard, sid, auth) = (
            Arc::clone(&shard),
            subscriber_id.clone(),
            shard_auth.clone(),
        );
        tokio::task::spawn_blocking(move || {
            shard.add_queued_datagram_subscriber(sid, &auth, datagram_max)
        })
        .await
        .unwrap_or_else(|e| Err(ShardError::Other(format!("subscribe task failed: {e}"))))
    };
    let queue = match joined {
        Ok(q) => q,
        Err(ShardError::Unauthorized(reason)) => {
            return close_after_notice(
                conn,
                &mut send,
                close_code::POLICY,
                &format!("unauthorized: {reason}"),
            )
            .await;
        }
        Err(e) => {
            return close_after_notice(conn, &mut send, close_code::AGAIN, &e.to_string()).await
        }
    };

    let wake = Arc::new(tokio::sync::Notify::new());
    {
        let wake = Arc::clone(&wake);
        queue.set_notifier(move || wake.notify_one());
    }
    let codec = wire::codec_byte(shard.snapshot_format());
    let writer = {
        let (queue, shard, conn) = (Arc::clone(&queue), Arc::clone(&shard), conn.clone());
        async move {
            let mut transferred: Option<String> = None;
            loop {
                while let Some(frame) = queue.pop() {
                    if frame.kind == FrameKind::Datagram {
                        // A datagram the connection cannot take now is lost,
                        // like one the network drops; the replicator sends
                        // its changes again.
                        if let Err(wtransport::error::SendDatagramError::NotConnected) =
                            conn.send_datagram(&frame.bytes[..])
                        {
                            return "the connection closed".to_string();
                        }
                        continue;
                    }
                    if frame.kind == FrameKind::Transfer {
                        transferred = serde_json::from_slice::<wire::TransferNotice>(&frame.bytes)
                            .map(|n| n.shard)
                            .ok()
                            .or(Some(String::new()));
                    }
                    if let Err(e) = write_message(&mut send, &frame_v2_of(&frame, codec)).await {
                        return e;
                    }
                }
                if queue.is_closed() {
                    let (code, reason) = match &transferred {
                        Some(to) => (close_code::NORMAL, format!("moved to shard {to}")),
                        None if shard.is_running() => {
                            (close_code::AGAIN, "client too slow".to_string())
                        }
                        None => (close_code::NORMAL, "shard stopped".to_string()),
                    };
                    close_after_notice(&conn, &mut send, code, &reason).await;
                    return reason;
                }
                wake.notified().await;
            }
        }
    };
    let mut writer = tokio::spawn(writer);

    let mut messages = StreamReader::spawn(recv);
    let reader = async {
        loop {
            tokio::select! {
                message = messages.next() => {
                    let bytes = match message {
                        Ok(Some(b)) => b,
                        Ok(None) => return ConnectionEnd::ClientClosed,
                        Err(e) => return read_failed(conn, e).await,
                    };
                    // QUIC's smoothed round trip, for lag compensation.
                    queue.set_rtt(conn.rtt());
                    match bytes.first() {
                        Some(&wire::client::ACKS) => match wire::decode_datagram_acks(&bytes) {
                            Some(acks) => queue.push_datagram_acks(&acks),
                            None => return close(conn, close_code::PROTOCOL, "malformed datagram acks"),
                        },
                        Some(&wire::client::INPUT) => {
                            process_input(&shard, &queue, &subscriber_id, &shard_auth,
                                shard.snapshot_format(), bytes[1..].to_vec(), 3).await;
                        }
                        Some(&wire::client::JSON_INPUT) => {
                            process_input(&shard, &queue, &subscriber_id, &shard_auth,
                                SnapshotFormat::Json, bytes[1..].to_vec(), 3).await;
                        }
                        _ => return close(conn, close_code::PROTOCOL, "unknown client message type"),
                    }
                }
                datagram = conn.receive_datagram() => {
                    let datagram = match datagram {
                        Ok(d) => d,
                        Err(e) => return ended_by(e),
                    };
                    queue.set_rtt(conn.rtt());
                    // Acks are the only datagrams a client sends; anything
                    // else is ignored, like a corrupt packet.
                    if let Some(acks) = wire::decode_datagram_acks(&datagram) {
                        queue.push_datagram_acks(&acks);
                    }
                }
                stopped = &mut writer => {
                    return ConnectionEnd::Closed(
                        stopped.unwrap_or_else(|e| format!("writer task failed: {e}")),
                    );
                }
            }
        }
    };
    let end = reader.await;
    shard.remove_queued_subscriber(&queue);
    writer.abort();
    end
}

/// Carry a session to the machine that runs its shard: a wire version 3
/// WebSocket to that machine's `/shard`, signed as a forwarded request (the
/// owner serves it and checks the client's credentials itself). Stream
/// frames go on to the session's stream, datagram frames go out as QUIC
/// datagrams without their header, and the client's stream messages and
/// datagrams (acks) go up as version 3 binary messages, which use the same
/// type bytes.
async fn relay(
    conn: &Connection,
    mut send: SendStream,
    recv: RecvStream,
    hello: &Hello,
    datagram_max: usize,
    machine_id: &str,
    address: &str,
) -> ConnectionEnd {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    let Some(host) = crate::shard_route::authority(address) else {
        return close(
            conn,
            close_code::AGAIN,
            "the shard's machine has no address",
        );
    };
    let target = format!(
        "/shard?shard={}&sid={}&v=3&dmax={datagram_max}",
        encode(&hello.shard),
        encode(&hello.sid)
    );
    let client_ip = conn.remote_address().ip().to_string();
    let Some(forwarded) =
        crate::shard_route::forwarded_value(&client_ip, "GET", &target, machine_id)
    else {
        return close_after_notice(
            conn,
            &mut send,
            close_code::AGAIN,
            "this machine is not in the shard directory",
        )
        .await;
    };
    let mut request = match format!("ws://{host}{target}").into_client_request() {
        Ok(r) => r,
        Err(e) => {
            return close_after_notice(
                conn,
                &mut send,
                close_code::AGAIN,
                &format!("relay request: {e}"),
            )
            .await
        }
    };
    let headers = request.headers_mut();
    let mut header = |name: &'static str, value: &str| {
        if let Ok(v) = value.parse() {
            headers.insert(name, v);
        }
    };
    header(crate::shard_cluster::FORWARDED_HEADER, &forwarded);
    if let Some(token) = &hello.token {
        header("authorization", &format!("Bearer {token}"));
    }
    if let Some(ticket) = &hello.ticket {
        header("x-pylon-shard-ticket", ticket);
    }
    let upstream = tokio::time::timeout(Duration::from_secs(5), async {
        let tcp = tokio::net::TcpStream::connect(host).await?;
        let _ = tcp.set_nodelay(true);
        tokio_tungstenite::client_async(request, tcp)
            .await
            .map_err(std::io::Error::other)
    })
    .await;
    let (ws, _) = match upstream {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            tracing::warn!("[shard-wt] relay to machine {machine_id} at {address}: {e}");
            return close_after_notice(
                conn,
                &mut send,
                close_code::AGAIN,
                "could not reach the shard's machine",
            )
            .await;
        }
        Err(_) => {
            return close_after_notice(
                conn,
                &mut send,
                close_code::AGAIN,
                "the shard's machine did not answer",
            )
            .await
        }
    };
    let (mut up_tx, mut up_rx) = ws.split();
    let mut messages = StreamReader::spawn(recv);
    loop {
        tokio::select! {
            from_owner = up_rx.next() => match from_owner {
                Some(Ok(Message::Binary(frame))) => {
                    if frame.len() >= wire::HEADER_LEN && frame[0] == wire::kind::DATAGRAM {
                        // Lost like any datagram when the connection is busy.
                        let _ = conn.send_datagram(&frame[wire::HEADER_LEN..]);
                    } else if let Err(e) = write_message(&mut send, &frame).await {
                        return ConnectionEnd::Closed(e);
                    }
                }
                Some(Ok(Message::Close(close_frame))) => {
                    let (code, reason) = match close_frame {
                        Some(f) => {
                            use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
                            let code = match f.code {
                                CloseCode::Policy => close_code::POLICY,
                                CloseCode::Protocol => close_code::PROTOCOL,
                                CloseCode::Again => close_code::AGAIN,
                                _ => close_code::NORMAL,
                            };
                            (code, f.reason.to_string())
                        }
                        None => (close_code::NORMAL, "the shard's machine closed".to_string()),
                    };
                    return close_after_notice(conn, &mut send, code, &reason).await;
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    return close_after_notice(conn, &mut send, close_code::AGAIN, &format!("relay: {e}"))
                        .await;
                }
                None => {
                    return close_after_notice(
                        conn,
                        &mut send,
                        close_code::AGAIN,
                        "the shard's machine closed",
                    )
                    .await
                }
            },
            message = messages.next() => match message {
                Ok(Some(bytes)) => {
                    if up_tx.send(Message::Binary(bytes)).await.is_err() {
                        return close_after_notice(
                            conn,
                            &mut send,
                            close_code::AGAIN,
                            "the shard's machine closed",
                        )
                        .await;
                    }
                }
                Ok(None) => {
                    let _ = up_tx.close().await;
                    return ConnectionEnd::ClientClosed;
                }
                Err(e) => {
                    let _ = up_tx.close().await;
                    return read_failed(conn, e).await;
                }
            },
            datagram = conn.receive_datagram() => match datagram {
                Ok(d) => {
                    if wire::decode_datagram_acks(&d).is_some()
                        && up_tx.send(Message::Binary(d.payload().to_vec())).await.is_err()
                    {
                        return close_after_notice(
                            conn,
                            &mut send,
                            close_code::AGAIN,
                            "the shard's machine closed",
                        )
                        .await;
                    }
                }
                Err(e) => {
                    let _ = up_tx.close().await;
                    return ended_by(e);
                }
            },
        }
    }
}

/// Percent-encode a query value (everything but unreserved characters).
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Close codes, for tests and clients: (normal, policy, protocol, again).
pub const CLOSE_CODES: (u32, u32, u32, u32) = (
    close_code::NORMAL,
    close_code::POLICY,
    close_code::PROTOCOL,
    close_code::AGAIN,
);
