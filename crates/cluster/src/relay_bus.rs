//! Managed cluster transport over the PylonSync Durable Object.
//!
//! Machines publish signed envelopes over HTTP and hold one hibernation-safe
//! WebSocket for inbound peer envelopes. The relay assigns a transport
//! sequence and keeps a bounded ring, so a transient socket reconnect can
//! replay missed frames. Postgres remains the data and change-log truth.

use crate::{new_instance_id, ClusterBus, Envelope, RelayFrame, SubscriberHandler};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info, warn};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

const PUBLISH_QUEUE_CAPACITY: usize = 16_384;
const MAX_BACKOFF_SECS: u64 = 30;

#[derive(Clone)]
struct RelayConfig {
    base_url: String,
    secret: String,
    app: String,
}

pub struct RelayBus {
    instance_id: String,
    sender: SyncSender<Envelope>,
    handlers: Arc<Mutex<Vec<SubscriberHandler>>>,
    last_relay_seq: Arc<AtomicU64>,
}

impl RelayBus {
    pub fn connect(base_url: &str, secret: &str, app: &str) -> Result<Self, String> {
        if base_url.trim().is_empty() || secret.is_empty() || app.trim().is_empty() {
            return Err("relay URL, secret, and app id are required".into());
        }
        let cfg = RelayConfig {
            base_url: base_url.trim_end_matches('/').to_string(),
            secret: secret.to_string(),
            app: app.to_string(),
        };
        let instance_id = new_instance_id();

        // Establish the subscriber before returning. A configured cluster bus
        // must fail at boot instead of silently running with local-only fanout.
        let socket = open_socket(&cfg, &instance_id, None)?;
        let handlers = Arc::new(Mutex::new(Vec::new()));
        let last_relay_seq = Arc::new(AtomicU64::new(0));
        let (sender, receiver) = sync_channel(PUBLISH_QUEUE_CAPACITY);

        {
            let cfg = cfg.clone();
            thread::Builder::new()
                .name("pylon-cluster-relay-pub".into())
                .spawn(move || run_publisher(receiver, cfg))
                .map_err(|e| format!("spawn relay publisher: {e}"))?;
        }
        {
            let cfg = cfg.clone();
            let handlers = Arc::clone(&handlers);
            let last_relay_seq = Arc::clone(&last_relay_seq);
            let subscriber_instance = instance_id.clone();
            thread::Builder::new()
                .name("pylon-cluster-relay-sub".into())
                .spawn(move || {
                    run_subscriber(cfg, subscriber_instance, socket, handlers, last_relay_seq)
                })
                .map_err(|e| format!("spawn relay subscriber: {e}"))?;
        }

        info!(
            "[cluster] durable relay connected — app={} instance_id={}",
            cfg.app, instance_id
        );
        Ok(Self {
            instance_id,
            sender,
            handlers,
            last_relay_seq,
        })
    }

    pub fn last_relay_seq(&self) -> u64 {
        self.last_relay_seq.load(Ordering::Relaxed)
    }
}

impl ClusterBus for RelayBus {
    fn publish(&self, envelope: &Envelope) {
        // Apply backpressure when the relay is unavailable. Dropping a committed
        // change would leave peer machines stale until a client reconciles.
        if self.sender.send(envelope.clone()).is_err() {
            error!("[cluster] relay publisher stopped; cluster delivery is unavailable");
        }
    }

    fn subscribe(&self, handler: SubscriberHandler) {
        self.handlers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(handler);
    }

    fn instance_id(&self) -> &str {
        &self.instance_id
    }
}

type RelaySocket = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

fn run_publisher(receiver: Receiver<Envelope>, cfg: RelayConfig) {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    while let Ok(envelope) = receiver.recv() {
        let message_id = uuid::Uuid::new_v4().to_string();
        let body = serde_json::json!({
            "app": cfg.app,
            "message_id": message_id,
            "envelope": envelope,
        })
        .to_string();
        let mut backoff = 1u64;
        loop {
            let timestamp = now_secs();
            let signature = pylon_auth::trusted_mint::sign(&cfg.secret, timestamp, body.as_bytes());
            let url = format!(
                "{}/sync/cluster/push?app={}",
                cfg.base_url,
                percent_encode(&cfg.app)
            );
            match agent
                .post(&url)
                .set("Content-Type", "application/json")
                .set("X-Pylon-Relay-Timestamp", &timestamp.to_string())
                .set("X-Pylon-Relay-Signature", &signature)
                .send_string(&body)
            {
                Ok(_) => break,
                Err(e) => {
                    warn!("[cluster] relay publish failed: {e}; retrying in {backoff}s");
                    thread::sleep(Duration::from_secs(backoff));
                    backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
                }
            }
        }
    }
}

fn run_subscriber(
    cfg: RelayConfig,
    instance_id: String,
    mut socket: RelaySocket,
    handlers: Arc<Mutex<Vec<SubscriberHandler>>>,
    last_relay_seq: Arc<AtomicU64>,
) {
    // The runtime constructs the bus before it installs its local fanout
    // handler. Keep inbound frames in the socket until that handler exists.
    // Advancing the replay cursor before a handler exists would lose events.
    while handlers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty()
    {
        thread::sleep(Duration::from_millis(10));
    }
    let mut backoff = 1u64;
    loop {
        match read_socket(&mut socket, &instance_id, &handlers, &last_relay_seq) {
            Ok(()) => warn!("[cluster] relay subscriber closed"),
            Err(e) => warn!("[cluster] relay subscriber ended: {e}"),
        }
        thread::sleep(Duration::from_secs(backoff));
        let since = last_relay_seq.load(Ordering::Relaxed);
        match open_socket(
            &cfg,
            &instance_id,
            if since == 0 { None } else { Some(since) },
        ) {
            Ok(next) => {
                socket = next;
                backoff = 1;
                info!("[cluster] relay subscriber reconnected at seq={since}");
            }
            Err(e) => {
                warn!("[cluster] relay reconnect failed: {e}");
                backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
            }
        }
    }
}

fn read_socket(
    socket: &mut RelaySocket,
    instance_id: &str,
    handlers: &Arc<Mutex<Vec<SubscriberHandler>>>,
    last_relay_seq: &AtomicU64,
) -> Result<(), String> {
    loop {
        match socket.read().map_err(|e| e.to_string())? {
            Message::Text(text) => {
                let frame: RelayFrame = match serde_json::from_str(&text) {
                    Ok(frame) => frame,
                    Err(e) => {
                        debug!("[cluster] malformed relay frame dropped: {e}");
                        continue;
                    }
                };
                last_relay_seq.fetch_max(frame.relay_seq, Ordering::Relaxed);
                if frame.envelope.instance_id == instance_id {
                    continue;
                }
                let snapshot: Vec<SubscriberHandler> = handlers
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .iter()
                    .map(Arc::clone)
                    .collect();
                for handler in snapshot {
                    let envelope = frame.envelope.clone();
                    if let Err(panic) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                            handler(envelope)
                        }))
                    {
                        error!("[cluster] relay subscriber handler panicked: {panic:?}");
                    }
                }
            }
            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .map_err(|e| e.to_string())?;
            }
            Message::Close(frame) => {
                if frame.as_ref().map(|f| u16::from(f.code)) == Some(4411) {
                    // The relay ring cannot cover the gap. Change events remain
                    // recoverable from Postgres; ephemeral presence restarts from
                    // current state. Reconnect without an impossible cursor.
                    last_relay_seq.store(0, Ordering::Relaxed);
                }
                return Ok(());
            }
            _ => {}
        }
    }
}

fn open_socket(
    cfg: &RelayConfig,
    instance_id: &str,
    since: Option<u64>,
) -> Result<RelaySocket, String> {
    let ws_base = if let Some(rest) = cfg.base_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = cfg.base_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        return Err("relay URL must start with http:// or https://".into());
    };
    let mut url = format!(
        "{ws_base}/sync/cluster/ws?app={}&instance={}",
        percent_encode(&cfg.app),
        percent_encode(instance_id)
    );
    if let Some(seq) = since {
        url.push_str(&format!("&since={seq}"));
    }
    let payload = format!("{}.{}", cfg.app, instance_id);
    let timestamp = now_secs();
    let signature = pylon_auth::trusted_mint::sign(&cfg.secret, timestamp, payload.as_bytes());
    let mut request = url
        .into_client_request()
        .map_err(|e| format!("relay websocket request: {e}"))?;
    request.headers_mut().insert(
        "X-Pylon-Relay-Timestamp",
        HeaderValue::from_str(&timestamp.to_string()).map_err(|e| e.to_string())?,
    );
    request.headers_mut().insert(
        "X-Pylon-Relay-Signature",
        HeaderValue::from_str(&signature).map_err(|e| e.to_string())?,
    );
    connect(request)
        .map(|(socket, _)| socket)
        .map_err(|e| format!("relay websocket connect: {e}"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_parts_are_encoded() {
        assert_eq!(percent_encode("project/a b"), "project%2Fa%20b");
    }
}
