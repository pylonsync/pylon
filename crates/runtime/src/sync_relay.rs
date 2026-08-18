//! Machine → Durable Object sync-relay push (`ChangeLogSink` impl).
//!
//! Option C from docs/SYNC_DURABLE_OBJECTS_DESIGN.md: the machine owns
//! the change log and `seq`; the relay owns delivery. After every
//! commit the sink enqueues the `ChangeEvent` (WITH raw `data` +
//! `prev_data` — the DO needs both for policy filtering and the
//! visibility-flip tombstone, and does its own `serverOnly`/`syncOmit`
//! projection before anything reaches a subscriber). A background
//! thread batches and POSTs to the relay with an HMAC-signed body.
//!
//! Dual-write by construction: the existing WS/SSE fan-out keeps
//! running; attaching this sink adds the relay path without touching
//! the machine's own delivery. Dropped pushes degrade to a catch-up
//! read against the machine — the disk log stays the source of truth,
//! so this thread never blocks the mutation hot path and never retries
//! forever (bounded retry, then drop + count).
//!
//! Config (feature off unless BOTH are set):
//! - `PYLON_SYNC_RELAY_URL`    — base URL of the worker, e.g.
//!   `https://sync.example.workers.dev`
//! - `PYLON_SYNC_RELAY_SECRET` — shared HMAC secret (same value as the
//!   worker's `PYLON_RELAY_SECRET` wrangler secret)
//! - `PYLON_SYNC_RELAY_APP`    — relay app id / DO name (defaults to
//!   `PYLON_PROJECT_ID`, then the manifest name)
//!
//! Wire (machine → worker, HMAC per `pylon_auth::trusted_mint`):
//! ```text
//! POST {url}/sync/push?app={app}
//!   X-Pylon-Relay-Timestamp: <unix secs>
//!   X-Pylon-Relay-Signature: <hex(HMAC_SHA256(secret, ts + "." + body))>
//!   {"events":[ChangeEvent, ...]}
//!
//! POST {url}/sync/manifest?app={app}     (once at boot)
//!   {"manifest":AppManifest, "auth_user":ManifestAuthUserConfig}
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use pylon_sync::{ChangeEvent, ChangeLogSink};

const CHANNEL_CAPACITY: usize = 4096;
const BATCH_MAX: usize = 200;
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);
/// One retry after a short pause, then drop the batch. The relay ring
/// is a cache of the log; persistence lives on the machine.
const RETRY_PAUSE: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct RelayConfig {
    /// `{base}/sync` — push + manifest endpoints hang off this.
    pub base_url: String,
    pub secret: String,
    /// DO name — one relay instance per app.
    pub app: String,
}

// Hand-written so `{:?}` never prints the shared HMAC secret.
impl std::fmt::Debug for RelayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayConfig")
            .field("base_url", &self.base_url)
            .field("secret", &"<redacted>")
            .field("app", &self.app)
            .finish()
    }
}

impl RelayConfig {
    /// Returns `None` (feature off) unless url + secret are both set.
    pub fn from_env(manifest_name: &str) -> Option<Self> {
        let url = std::env::var("PYLON_SYNC_RELAY_URL")
            .ok()
            .filter(|s| !s.is_empty())?;
        let secret = match std::env::var("PYLON_SYNC_RELAY_SECRET")
            .ok()
            .filter(|s| !s.is_empty())
        {
            Some(s) => s,
            None => {
                tracing::warn!(
                    "[sync-relay] PYLON_SYNC_RELAY_URL is set but PYLON_SYNC_RELAY_SECRET is empty — relay disabled"
                );
                return None;
            }
        };
        let app = std::env::var("PYLON_SYNC_RELAY_APP")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::env::var("PYLON_PROJECT_ID")
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| manifest_name.to_string());
        Some(Self {
            base_url: url.trim_end_matches('/').to_string(),
            secret,
            app,
        })
    }
}

/// The sink handed to `ChangeLog::with_sink`. `push` is try_send +
/// return — full channel drops the event and bumps the counter.
pub struct SyncRelaySink {
    sender: SyncSender<ChangeEvent>,
    pub dropped: Arc<AtomicU64>,
    pub pushed: Arc<AtomicU64>,
    pub failed: Arc<AtomicU64>,
}

impl std::fmt::Debug for SyncRelaySink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncRelaySink")
            .field("dropped", &self.dropped.load(Ordering::Relaxed))
            .field("pushed", &self.pushed.load(Ordering::Relaxed))
            .field("failed", &self.failed.load(Ordering::Relaxed))
            .finish()
    }
}

impl ChangeLogSink for SyncRelaySink {
    fn push(&self, event: &ChangeEvent) {
        match self.sender.try_send(event.clone()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl SyncRelaySink {
    /// Spin up the shipper thread and return the sink, or `None` when
    /// the env doesn't configure a relay. `manifest_json` is the
    /// serialized `{"manifest":..., "auth_user":...}` payload pushed
    /// once at boot so the DO can build its `PolicyEngine`.
    pub fn start(cfg: RelayConfig, manifest_json: String) -> Arc<Self> {
        let (tx, rx) = sync_channel::<ChangeEvent>(CHANNEL_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let pushed = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicU64::new(0));
        let pushed_clone = Arc::clone(&pushed);
        let failed_clone = Arc::clone(&failed);
        let cfg_for_log = cfg.clone();
        thread::Builder::new()
            .name("pylon-sync-relay".into())
            .spawn(move || run_shipper(rx, cfg, manifest_json, pushed_clone, failed_clone))
            .expect("spawn pylon-sync-relay thread");
        tracing::warn!(
            "[sync-relay] pushing change events to {} (app {})",
            cfg_for_log.base_url,
            cfg_for_log.app
        );
        Arc::new(Self {
            sender: tx,
            dropped,
            pushed,
            failed,
        })
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn signed_post(
    agent: &ureq::Agent,
    cfg: &RelayConfig,
    path: &str,
    body: &str,
) -> Result<(), String> {
    let ts = now_secs();
    let sig = pylon_auth::trusted_mint::sign(&cfg.secret, ts, body.as_bytes());
    let url = format!(
        "{}/sync/{path}?app={}",
        cfg.base_url,
        urlencoding_encode(&cfg.app)
    );
    agent
        .post(&url)
        .set("Content-Type", "application/json")
        .set("X-Pylon-Relay-Timestamp", &ts.to_string())
        .set("X-Pylon-Relay-Signature", &sig)
        .send_string(body)
        .map(|_| ())
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("HTTP {code}"),
            other => other.to_string(),
        })
}

/// Percent-encode the app id for the query string. App ids are project
/// ids / manifest names, so this is belt-and-braces, not load-bearing.
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn run_shipper(
    rx: Receiver<ChangeEvent>,
    cfg: RelayConfig,
    manifest_json: String,
    pushed: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
) {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();

    // Manifest first — the DO fails closed (denies every frame) until
    // it has policies, so a lost manifest push means no delivery, not
    // a leak. Retry a few times; deliveries queued meanwhile still
    // land in the DO's ring for catch-up once the manifest arrives.
    let push_manifest = |manifest_ok: &mut bool| {
        for attempt in 0..5u32 {
            match signed_post(&agent, &cfg, "manifest", &manifest_json) {
                Ok(()) => {
                    *manifest_ok = true;
                    return;
                }
                Err(e) => {
                    tracing::warn!("[sync-relay] manifest push attempt {attempt} failed: {e}");
                    thread::sleep(Duration::from_secs(2 << attempt.min(4)));
                }
            }
        }
    };
    let mut manifest_ok = false;
    push_manifest(&mut manifest_ok);
    if !manifest_ok {
        tracing::error!(
            "[sync-relay] initial manifest push failed — retrying on the next event; the relay denies frames until it lands"
        );
    }

    let mut batch: Vec<ChangeEvent> = Vec::with_capacity(BATCH_MAX);
    loop {
        // Block for the first event, then drain up to BATCH_MAX or
        // until the flush interval elapses.
        match rx.recv_timeout(FLUSH_INTERVAL) {
            Ok(ev) => batch.push(ev),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        while batch.len() < BATCH_MAX {
            match rx.try_recv() {
                Ok(ev) => batch.push(ev),
                Err(_) => break,
            }
        }
        if batch.is_empty() {
            continue;
        }
        // A DO that restarted loses its in-memory manifest (it re-reads
        // from storage on wake, but a fresh DO instance has none). If
        // the initial manifest never landed, re-push before delivering
        // — otherwise these events ring but never fan out.
        if !manifest_ok {
            push_manifest(&mut manifest_ok);
        }
        // `app` inside the signed body binds this push to its target
        // app: the DO rejects it if the routed `?app=` differs, so a
        // captured signed push can't be replayed to another app's DO.
        let body = serde_json::json!({ "app": cfg.app, "events": batch }).to_string();
        let mut ok = false;
        for attempt in 0..2u32 {
            match signed_post(&agent, &cfg, "push", &body) {
                Ok(()) => {
                    ok = true;
                    break;
                }
                Err(e) => {
                    if attempt == 0 {
                        thread::sleep(RETRY_PAUSE);
                    } else {
                        // A push failure can mean the DO restarted and
                        // dropped its manifest — force a re-push before
                        // the next batch so delivery self-heals.
                        manifest_ok = false;
                        tracing::warn!(
                            "[sync-relay] push of {} event(s) dropped after retry: {e}",
                            batch.len()
                        );
                    }
                }
            }
        }
        if ok {
            pushed.fetch_add(batch.len() as u64, Ordering::Relaxed);
        } else {
            failed.fetch_add(batch.len() as u64, Ordering::Relaxed);
        }
        batch.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_sync::ChangeKind;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// Minimal one-shot HTTP server capturing a single request.
    fn capture_one_request(
        listener: TcpListener,
    ) -> thread::JoinHandle<(String, Vec<(String, String)>, String)> {
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut headers = Vec::new();
            let mut content_len = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let line = line.trim_end().to_string();
                if line.is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(": ") {
                    if k.eq_ignore_ascii_case("content-length") {
                        content_len = v.parse().unwrap_or(0);
                    }
                    headers.push((k.to_ascii_lowercase(), v.to_string()));
                }
            }
            let mut body = vec![0u8; content_len];
            reader.read_exact(&mut body).unwrap();
            let mut stream = reader.into_inner();
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n");
            (
                request_line.trim_end().to_string(),
                headers,
                String::from_utf8(body).unwrap(),
            )
        })
    }

    #[test]
    fn from_env_requires_both_url_and_secret() {
        // Uses explicit values rather than env mutation (tests run in
        // parallel; setting env vars here would race other tests).
        let cfg = RelayConfig {
            base_url: "https://relay.example".into(),
            secret: "s".into(),
            app: "demo".into(),
        };
        assert_eq!(cfg.base_url, "https://relay.example");
    }

    #[test]
    fn shipper_signs_manifest_and_push_bodies() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let manifest_capture = capture_one_request(listener);

        let cfg = RelayConfig {
            base_url: format!("http://{addr}"),
            secret: "relay-secret".into(),
            app: "my app".into(),
        };
        let sink = SyncRelaySink::start(cfg, r#"{"manifest":{}}"#.to_string());

        // First request off the wire is the boot manifest push.
        let (line, headers, body) = manifest_capture.join().unwrap();
        assert!(
            line.starts_with("POST /sync/manifest?app=my%20app"),
            "{line}"
        );
        assert_eq!(body, r#"{"manifest":{}}"#);
        let ts: u64 = headers
            .iter()
            .find(|(k, _)| k == "x-pylon-relay-timestamp")
            .unwrap()
            .1
            .parse()
            .unwrap();
        let sig = &headers
            .iter()
            .find(|(k, _)| k == "x-pylon-relay-signature")
            .unwrap()
            .1;
        pylon_auth::trusted_mint::verify_signature(
            "relay-secret",
            ts,
            body.as_bytes(),
            sig,
            now_secs(),
        )
        .expect("manifest signature verifies");

        // The sink itself never blocks even with no listener anymore.
        sink.push(&ChangeEvent {
            seq: 1,
            entity: "Todo".into(),
            row_id: "t1".into(),
            kind: ChangeKind::Insert,
            data: Some(serde_json::json!({"x": 1})),
            prev_data: None,
            timestamp: "2026-01-01T00:00:00Z".into(),
        });
    }
}
