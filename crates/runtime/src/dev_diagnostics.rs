//! Dev-only diagnostics ring — the machine-readable side of the dev HUD.
//!
//! The visual HUD (an overlay injected into the page) answers "why isn't this
//! caching / what's the render cost" for a human. A coding AGENT building a Pylon
//! app needs the same answers but can't see an overlay, so the framework records
//! each SSR render's verdict here and serves it at `GET /_pylon/dev/diagnostics`
//! (+ the `pylon diagnostics` CLI). The verdict + reason are computed in the Bun
//! runtime (`describeCacheVerdict`) and ride to Rust on the trusted, dev-only
//! `x-pylon-dev` response header (stripped before it reaches the client), so this
//! needs no new protocol frame.
//!
//! Bounded in-memory ring (newest wins); only written under `is_dev_mode()`, so
//! production never allocates it.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// How many recent renders to keep. Small — an agent reads the tail.
const CAP: usize = 100;

/// One SSR render's diagnostics, mirroring the dev HUD blob.
#[derive(Clone, serde::Serialize)]
pub struct SsrEvent {
    /// Epoch milliseconds the render completed.
    pub ts_ms: u128,
    /// Request path (no query — bucket cache key is path-only).
    pub route: String,
    /// Resolved component (e.g. `app/page`).
    pub component: String,
    /// `cacheable` | `bucketed` | `dynamic`.
    pub verdict: String,
    /// Revalidate TTL when cacheable/bucketed.
    pub secs: Option<u64>,
    /// The single most actionable reason (esp. WHY a render is `dynamic`).
    pub reason: String,
    /// `ssr-buffered` | `ssr-streaming`.
    pub mode: String,
    /// Wall-clock render time, milliseconds (Rust-measured, end to end).
    pub render_ms: f64,
    /// HTTP status the render emitted.
    pub status: u16,
}

static RING: Mutex<VecDeque<SsrEvent>> = Mutex::new(VecDeque::new());

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Record an SSR render. Caller supplies everything except the timestamp.
#[allow(clippy::too_many_arguments)]
pub fn record_ssr(
    route: String,
    component: String,
    verdict: String,
    secs: Option<u64>,
    reason: String,
    mode: String,
    render_ms: f64,
    status: u16,
) {
    if let Ok(mut r) = RING.lock() {
        if r.len() >= CAP {
            r.pop_front();
        }
        r.push_back(SsrEvent {
            ts_ms: now_ms(),
            route,
            component,
            verdict,
            secs,
            reason,
            mode,
            render_ms,
            status,
        });
    }
}

/// The whole ring as a JSON body for `GET /_pylon/dev/diagnostics`. Newest last
/// (chronological), so an agent reading the tail sees its most recent renders.
pub fn snapshot_json() -> String {
    let events: Vec<SsrEvent> = RING
        .lock()
        .map(|r| r.iter().cloned().collect())
        .unwrap_or_default();
    serde_json::json!({ "ssr": events }).to_string()
}

/// Parse the dev-only `x-pylon-dev` header (JSON the runtime emits in dev) and
/// record it with the Rust-measured timing/status. Returns the human-readable
/// one-line summary (for the structured dev log), or `None` if absent/malformed.
pub fn record_from_header(
    header_value: Option<&str>,
    route: &str,
    status: u16,
    render_ms: f64,
) -> Option<String> {
    let raw = header_value?;
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let verdict = v
        .get("verdict")
        .and_then(|x| x.as_str())
        .unwrap_or("dynamic");
    let reason = v.get("reason").and_then(|x| x.as_str()).unwrap_or("");
    let mode = v.get("mode").and_then(|x| x.as_str()).unwrap_or("ssr");
    let component = v.get("component").and_then(|x| x.as_str()).unwrap_or("");
    let secs = v.get("secs").and_then(|x| x.as_u64());
    record_ssr(
        route.to_string(),
        component.to_string(),
        verdict.to_string(),
        secs,
        reason.to_string(),
        mode.to_string(),
        render_ms,
        status,
    );
    let ttl = secs.map(|s| format!(" {s}s")).unwrap_or_default();
    Some(format!(
        "{verdict}{ttl} · {render_ms:.1}ms{}",
        if reason.is_empty() {
            String::new()
        } else {
            format!(" — {reason}")
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_from_header_parses_and_summarizes() {
        // A well-formed x-pylon-dev header records an event + returns the summary.
        let hdr = r#"{"verdict":"dynamic","secs":null,"reason":"read props.auth","mode":"ssr-buffered","component":"app/page"}"#;
        let summary = record_from_header(Some(hdr), "/account", 200, 12.3);
        let summary = summary.expect("well-formed header yields a summary");
        assert!(summary.contains("dynamic"));
        assert!(summary.contains("12.3ms"));
        assert!(summary.contains("read props.auth"));
        // It landed in the ring with the Rust-measured timing/status/route.
        let snap = snapshot_json();
        assert!(snap.contains("\"route\":\"/account\""));
        assert!(snap.contains("\"render_ms\":12.3"));
        assert!(snap.contains("\"status\":200"));
    }

    #[test]
    fn record_from_header_is_fail_closed_on_missing_or_malformed() {
        assert!(record_from_header(None, "/x", 200, 1.0).is_none());
        assert!(record_from_header(Some("not json"), "/x", 200, 1.0).is_none());
        // A cacheable verdict carries its TTL into the summary.
        let hdr = r#"{"verdict":"cacheable","secs":60,"reason":"anonymous shared cache","mode":"ssr-buffered"}"#;
        let s = record_from_header(Some(hdr), "/blog", 200, 3.0).unwrap();
        assert!(s.contains("cacheable 60s"));
    }
}
