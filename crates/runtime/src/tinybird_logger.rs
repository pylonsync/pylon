//! Optional Tinybird request-log shipper.
//!
//! When `PYLON_TINYBIRD_TOKEN` is set in the environment, every completed
//! HTTP request emits one NDJSON row to the configured Tinybird
//! datasource. Pylon Cloud uses this to feed the per-project Logs page
//! on the dashboard; standalone Pylon installs ignore the feature
//! entirely when the env isn't set.
//!
//! Design constraints:
//! - **Non-blocking on the request path.** `record_request()` is on the
//!   critical hot path. The emitter is a bounded `mpsc::sync_channel`
//!   with `try_send` — when the buffer is full, events are dropped (and
//!   counted) rather than blocking the request thread.
//! - **One background flusher thread.** Owns the HTTP client + a small
//!   pending Vec; flushes when the batch hits `BATCH_MAX` events OR the
//!   flush interval elapses. No tokio dep — tiny_http is sync.
//! - **No retry hell.** A failed POST is logged once via tracing and
//!   the batch is dropped. Pylon's job queue + Tinybird's own ingest
//!   resilience cover the durability gap; we're shipping operational
//!   logs, not financial events.

use serde::Serialize;
use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Single request-log row. Field names match the Tinybird `request_log`
/// datasource schema 1:1 (the .datasource file in pylon-cloud's repo
/// is the source of truth — keep these aligned).
#[derive(Debug, Clone, Serialize)]
pub struct RequestLogEvent {
    pub timestamp: String,
    pub project_id: String,
    pub deployment_id: String,
    pub path: String,
    pub method: String,
    pub status: u16,
    pub cpu_ms: u32,
    pub bytes_in: u32,
    pub bytes_out: u32,
    pub region: String,
    pub error: String,
}

/// Config resolved from env at startup. `None` here means the shipper
/// is disabled — the most common case (any non-cloud deploy).
#[derive(Debug, Clone)]
struct ResolvedConfig {
    host: String,
    token: String,
    project_id: String,
    deployment_id: String,
    region: String,
    datasource: String,
}

impl ResolvedConfig {
    fn from_env() -> Option<Self> {
        let token = env::var("PYLON_TINYBIRD_TOKEN").ok()?;
        if token.is_empty() {
            return None;
        }
        // Project id is required — without it every row would land in
        // the same bucket and the dashboard couldn't filter per
        // tenant. Refuse to start the shipper rather than ship rows
        // that the dashboard can't index.
        let project_id = env::var("PYLON_PROJECT_ID").ok()?;
        if project_id.is_empty() {
            tracing::warn!(
                "[tinybird] PYLON_TINYBIRD_TOKEN set but PYLON_PROJECT_ID is empty — shipper disabled"
            );
            return None;
        }
        let host = env::var("PYLON_TINYBIRD_HOST")
            .unwrap_or_else(|_| "https://api.tinybird.co".to_string());
        let deployment_id = env::var("PYLON_DEPLOYMENT_ID").unwrap_or_default();
        // Prefer the explicit PYLON_REGION override; fall back to
        // FLY_REGION which is auto-set on Fly machines. Empty string
        // is fine — Tinybird's `LowCardinality(String)` accepts it.
        let region = env::var("PYLON_REGION")
            .or_else(|_| env::var("FLY_REGION"))
            .unwrap_or_default();
        let datasource =
            env::var("PYLON_TINYBIRD_DATASOURCE").unwrap_or_else(|_| "request_log".to_string());
        Some(Self {
            host,
            token,
            project_id,
            deployment_id,
            region,
            datasource,
        })
    }
}

const CHANNEL_CAPACITY: usize = 2048;
const BATCH_MAX: usize = 100;
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);

/// Handle returned from [`init`] when the env is configured. Holds
/// the sender end of the work channel + counters for observability.
pub struct TinybirdLogger {
    sender: SyncSender<RequestLogEvent>,
    pub dropped: Arc<AtomicU64>,
    pub flushed: Arc<AtomicU64>,
    pub failed: Arc<AtomicU64>,
    /// Static env-derived fields stamped on every event so `record()`
    /// callers don't have to know about them.
    project_id: String,
    deployment_id: String,
    region: String,
}

impl TinybirdLogger {
    /// Try to spin up a shipper from env. Returns `None` if the env
    /// isn't configured — callers should treat that as "feature off"
    /// rather than an error.
    pub fn from_env() -> Option<Arc<Self>> {
        let cfg = ResolvedConfig::from_env()?;
        let (tx, rx) = sync_channel::<RequestLogEvent>(CHANNEL_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let flushed = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicU64::new(0));

        let flushed_clone = Arc::clone(&flushed);
        let failed_clone = Arc::clone(&failed);
        let cfg_clone = cfg.clone();
        thread::Builder::new()
            .name("pylon-tinybird-logger".into())
            .spawn(move || run_flusher(rx, cfg_clone, flushed_clone, failed_clone))
            .expect("spawn pylon-tinybird-logger thread");

        tracing::info!(
            "[tinybird] request-log shipper started → {} / {} (project_id={})",
            cfg.host,
            cfg.datasource,
            cfg.project_id
        );
        Some(Arc::new(Self {
            sender: tx,
            dropped,
            flushed,
            failed,
            project_id: cfg.project_id,
            deployment_id: cfg.deployment_id,
            region: cfg.region,
        }))
    }

    /// Best-effort enqueue. Returns immediately; on backpressure the
    /// event is dropped and `dropped` is incremented. Callers MUST NOT
    /// block here — this runs on every HTTP response.
    pub fn record(
        &self,
        method: &str,
        path: &str,
        status: u16,
        cpu_ms: u32,
        bytes_in: u32,
        bytes_out: u32,
        error: &str,
    ) {
        let event = RequestLogEvent {
            timestamp: iso_now_ms(),
            project_id: self.project_id.clone(),
            deployment_id: self.deployment_id.clone(),
            path: path.to_string(),
            method: method.to_string(),
            status,
            cpu_ms,
            bytes_in,
            bytes_out,
            region: self.region.clone(),
            error: error.to_string(),
        };
        match self.sender.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                // Flusher thread died — silently drop. We'd rather lose
                // logs than crash the request path.
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn run_flusher(
    rx: std::sync::mpsc::Receiver<RequestLogEvent>,
    cfg: ResolvedConfig,
    flushed: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
) {
    let url = format!("{}/v0/events?name={}", cfg.host, cfg.datasource);
    let auth_header = format!("Bearer {}", cfg.token);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let mut buf: Vec<RequestLogEvent> = Vec::with_capacity(BATCH_MAX);
    let mut last_flush = Instant::now();
    loop {
        // Drain as many events as available or wait up to the flush
        // interval for the next one. Channel disconnect = shutdown.
        let remaining = FLUSH_INTERVAL.saturating_sub(last_flush.elapsed());
        let wait_for = if buf.is_empty() {
            // Empty buffer — block indefinitely until a row arrives so
            // an idle process doesn't spin.
            Duration::from_secs(60)
        } else if remaining.is_zero() {
            Duration::from_millis(0)
        } else {
            remaining
        };

        match rx.recv_timeout(wait_for) {
            Ok(event) => {
                buf.push(event);
                if buf.len() >= BATCH_MAX {
                    flush_batch(&agent, &url, &auth_header, &mut buf, &flushed, &failed);
                    last_flush = Instant::now();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !buf.is_empty() {
                    flush_batch(&agent, &url, &auth_header, &mut buf, &flushed, &failed);
                    last_flush = Instant::now();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if !buf.is_empty() {
                    flush_batch(&agent, &url, &auth_header, &mut buf, &flushed, &failed);
                }
                tracing::info!("[tinybird] sender disconnected, shipper exiting");
                return;
            }
        }
    }
}

fn flush_batch(
    agent: &ureq::Agent,
    url: &str,
    auth_header: &str,
    buf: &mut Vec<RequestLogEvent>,
    flushed: &AtomicU64,
    failed: &AtomicU64,
) {
    if buf.is_empty() {
        return;
    }
    // NDJSON: one JSON object per line, no trailing comma. Tinybird's
    // /v0/events endpoint expects this exact format.
    let mut body = String::with_capacity(buf.len() * 256);
    for ev in buf.iter() {
        match serde_json::to_string(ev) {
            Ok(line) => {
                body.push_str(&line);
                body.push('\n');
            }
            Err(e) => {
                tracing::warn!("[tinybird] dropping event: serialize failed: {e}");
            }
        }
    }
    let n = buf.len();
    buf.clear();
    let result = agent
        .post(url)
        .set("Authorization", auth_header)
        .set("Content-Type", "application/x-ndjson")
        .send_string(&body);
    match result {
        Ok(_) => {
            flushed.fetch_add(n as u64, Ordering::Relaxed);
        }
        Err(e) => {
            failed.fetch_add(n as u64, Ordering::Relaxed);
            tracing::warn!("[tinybird] flush failed ({n} events dropped): {e}");
        }
    }
}

/// Cheap ISO-8601 timestamp with millisecond precision. Tinybird's
/// `DateTime64(3)` parser accepts this format directly.
fn iso_now_ms() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let millis = d.subsec_millis();
    // chrono is not a direct dep; format manually. UTC.
    let (year, month, day, hour, minute, second) = secs_to_ymd_hms(secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hour, minute, second, millis
    )
}

fn secs_to_ymd_hms(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    // Days since 1970-01-01.
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hour = (rem / 3600) as u32;
    let minute = ((rem % 3600) / 60) as u32;
    let second = (rem % 60) as u32;
    let (year, month, day) = civil_from_days(days);
    (year, month, day, hour, minute, second)
}

/// Howard Hinnant's date algorithm: days since 1970-01-01 → (y, m, d).
/// Public-domain reference implementation.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_format_matches_datetime64() {
        let s = iso_now_ms();
        // 2026-05-11T22:30:24.407Z — 24 chars.
        assert_eq!(s.len(), 24, "got {s}");
        assert!(s.ends_with('Z'));
        assert!(s.contains('T'));
        // Millisecond component must be exactly 3 digits.
        let dot = s.rfind('.').unwrap();
        assert_eq!(&s[dot + 1..s.len() - 1].len(), &3);
    }

    #[test]
    fn civil_from_days_epoch() {
        // 1970-01-01.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn civil_from_days_known_dates() {
        // 2000-01-01 = day 10957 (Howard Hinnant's reference table).
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        // 2024-02-29 = day 19782 (leap day; well-known checkpoint).
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        // Round-trip a recent timestamp through both helpers and
        // verify the output matches the date we'd compute manually.
        // 2026-01-01: 56 years from 1970 with 14 leap years.
        assert_eq!(civil_from_days(56 * 365 + 14), (2026, 1, 1));
    }

    #[test]
    fn from_env_returns_none_when_unset() {
        // Use std::env::remove_var directly — tests in this crate set
        // PYLON_TINYBIRD_TOKEN to "" elsewhere, but a removed var is
        // the most common deployment state.
        // Note: this test is best-effort; if another test sets the
        // env in parallel we'll get a false negative.
        let prev = env::var("PYLON_TINYBIRD_TOKEN").ok();
        env::remove_var("PYLON_TINYBIRD_TOKEN");
        let cfg = ResolvedConfig::from_env();
        if let Some(v) = prev {
            env::set_var("PYLON_TINYBIRD_TOKEN", v);
        }
        assert!(cfg.is_none(), "expected None when env is unset");
    }
}
