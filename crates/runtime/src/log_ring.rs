//! In-process request-log ring buffer for the `/admin/logs/tail`
//! endpoint.
//!
//! The Tinybird shipper writes every request to an external
//! analytics store — great for long-term aggregates, terrible as
//! a live-tail backend (each query costs a Tinybird quota credit
//! and adds 50-300ms of network latency). The dashboard's logs
//! page polls every 2s, which on Tinybird's free tier exhausts
//! the daily quota in ~22 minutes.
//!
//! Solution: keep a small in-process ring of the most recent
//! request log entries, and serve the tail directly from it. The
//! ring is bounded (oldest entries drop off), so it's not a
//! durable log — Tinybird is still the source of truth for
//! anything more than `RING_CAPACITY` deep. But for "what just
//! happened on this machine?" — the dashboard's primary use
//! case — the ring is free, lock-cheap, and gives sub-second
//! freshness.
//!
//! Lives at module scope as a `OnceLock`, mirroring the
//! TinybirdLogger pattern. One ring per process; the dashboard
//! talks to one customer machine at a time (Pylon Cloud is
//! single-machine per project today).

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// One captured request line — same shape the Tinybird shipper
/// emits, minus the deployment_id/project_id/region fields the
/// dashboard already knows from its caller-side context. Keeping
/// the in-process shape lean lets the ring hold more entries for
/// the same memory budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RingEntry {
    /// ISO-8601 with millisecond precision, UTC. Same format the
    /// Tinybird shipper produces — the dashboard uses this as the
    /// poll cursor (`?since=`).
    pub timestamp: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub cpu_ms: u32,
    /// Request and response body sizes in bytes.
    pub bytes_in: u32,
    pub bytes_out: u32,
    /// The id of the signed-in caller. Omitted for an anonymous request and
    /// for a request that ended before the server resolved the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// The request is one of Studio's own: a Studio page, the admin API, or
    /// an API call that Studio made. A log view can leave these out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub studio: bool,
    /// Why the request or connection failed. Omitted when it did not fail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The number of requests the ring keeps when `PYLON_REQUEST_LOG_SIZE` is
/// not set. At about 200 bytes an entry, 1,000 entries use about 200 KB.
const RING_CAPACITY: usize = 1000;
/// The bounds of `PYLON_REQUEST_LOG_SIZE`.
const MIN_CAPACITY: usize = 100;
const MAX_CAPACITY: usize = 50_000;

/// `PYLON_REQUEST_LOG_SIZE`, clamped to 100..=50,000; 1,000 when unset or
/// not a number.
pub fn capacity_from_env() -> usize {
    std::env::var("PYLON_REQUEST_LOG_SIZE")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|n| n.clamp(MIN_CAPACITY, MAX_CAPACITY))
        .unwrap_or(RING_CAPACITY)
}

/// Lock-guarded ring buffer. `record` runs once per request — the
/// contention is the same as the `request_buckets` Mutex already
/// held in `Metrics::record_request`, and we get the same
/// "dwarfed by everything else in dispatch" amortization.
pub struct RequestLogRing {
    inner: Mutex<VecDeque<RingEntry>>,
    capacity: usize,
    /// Entries pushed since the process started. The file writer compares
    /// it with what it wrote to find the new entries.
    pushed: AtomicU64,
}

impl Default for RequestLogRing {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestLogRing {
    pub fn new() -> Self {
        Self::with_capacity(RING_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(capacity.min(MAX_CAPACITY))),
            capacity,
            pushed: AtomicU64::new(0),
        }
    }

    /// The most entries the ring keeps.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Append a new entry, dropping the oldest when at capacity.
    /// Best-effort: a poisoned mutex (panic in another thread holding
    /// the lock) silently drops the entry rather than propagating —
    /// the ring is observability infrastructure and must never take
    /// down the request hot path.
    pub fn push(&self, entry: RingEntry) {
        if let Ok(mut q) = self.inner.lock() {
            if q.len() >= self.capacity {
                q.pop_front();
            }
            q.push_back(entry);
            self.pushed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The entries pushed after the first `written` pushes, oldest first,
    /// and the push count they reach. When more than the ring holds were
    /// pushed, the older ones are gone and only the ring is returned.
    fn since_push(&self, written: u64) -> (Vec<RingEntry>, u64) {
        let Ok(q) = self.inner.lock() else {
            return (Vec::new(), written);
        };
        let pushed = self.pushed.load(Ordering::Relaxed);
        let new = (pushed.saturating_sub(written) as usize).min(q.len());
        (q.iter().skip(q.len() - new).cloned().collect(), pushed)
    }

    /// Every entry in the ring, oldest first (restored ones included), and
    /// the push count it reaches. Read under one lock, so the two agree.
    fn snapshot(&self) -> (Vec<RingEntry>, u64) {
        let Ok(q) = self.inner.lock() else {
            return (Vec::new(), self.pushed.load(Ordering::Relaxed));
        };
        (
            q.iter().cloned().collect(),
            self.pushed.load(Ordering::Relaxed),
        )
    }

    /// Put entries read back from the file at the start of the ring, without
    /// counting them as new.
    fn restore(&self, entries: Vec<RingEntry>) {
        if let Ok(mut q) = self.inner.lock() {
            let room = self.capacity.saturating_sub(q.len());
            let skip = entries.len().saturating_sub(room);
            for e in entries.into_iter().skip(skip).rev() {
                q.push_front(e);
            }
        }
    }

    /// Return all entries with `timestamp > since` (strict, mirrors
    /// the Tinybird pipe's semantics so the dashboard cursor logic
    /// doesn't need a branch for backend choice). `since` is a
    /// lexicographic compare on the ISO-8601 string — valid because
    /// ISO-8601 timestamps sort identically as strings and as
    /// real-time. Pass `None` for a full snapshot.
    ///
    /// Returns newest-first to match the existing `recent_logs`
    /// pipe order, so the dashboard's `[...rows].reverse()` and
    /// cursor-on-`rows[0]` logic still works unchanged.
    pub fn tail_since(&self, since: Option<&str>) -> Vec<RingEntry> {
        let q = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<RingEntry> = match since {
            Some(s) => q
                .iter()
                .filter(|e| e.timestamp.as_str() > s)
                .cloned()
                .collect(),
            None => q.iter().cloned().collect(),
        };
        out.reverse();
        out
    }
}

// Process-global ring. Same lazy-init pattern as TINYBIRD_LOGGER so
// callers don't have to wire it through; one `init` call at server
// boot, then `record_request` reads via `log_ring()`.
static LOG_RING: OnceLock<RequestLogRing> = OnceLock::new();

pub fn init_log_ring() {
    LOG_RING.get_or_init(|| RequestLogRing::with_capacity(capacity_from_env()));
}

// ---------------------------------------------------------------------------
// The request log file
// ---------------------------------------------------------------------------
//
// The ring is in memory. A writer thread appends the new entries to a JSON
// Lines file every few seconds, and the next process reads the file back, so
// a restart or a deploy keeps the log. The file is rewritten from the ring
// when it holds twice the ring's capacity, so it stays bounded. A crash loses
// at most the last few seconds.

/// How often the writer appends new entries.
const FLUSH_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

struct LogFile {
    path: PathBuf,
    /// Pushes already in the file.
    written: u64,
    /// Lines in the file.
    lines: usize,
}

static LOG_FILE: Mutex<Option<LogFile>> = Mutex::new(None);

/// Read the last `capacity` entries of a request log file. A line that does
/// not parse is skipped.
fn read_file(path: &Path, capacity: usize) -> (Vec<RingEntry>, usize) {
    let Ok(file) = std::fs::File::open(path) else {
        return (Vec::new(), 0);
    };
    let mut entries = VecDeque::with_capacity(capacity.min(MAX_CAPACITY));
    let mut lines = 0;
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        lines += 1;
        if let Ok(e) = serde_json::from_str::<RingEntry>(&line) {
            if entries.len() >= capacity {
                entries.pop_front();
            }
            entries.push_back(e);
        }
    }
    (entries.into(), lines)
}

fn write_lines(path: &Path, entries: &[RingEntry], append: bool) -> std::io::Result<()> {
    let mut out = String::new();
    for e in entries {
        if let Ok(line) = serde_json::to_string(e) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    if append {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        f.write_all(out.as_bytes())
    } else {
        let tmp = path.with_extension("jsonl.tmp");
        std::fs::write(&tmp, out.as_bytes())?;
        std::fs::rename(&tmp, path)
    }
}

/// Write the entries pushed since the last write. Rewrites the file from the
/// ring when the file would hold more than twice the ring's capacity.
pub fn flush() {
    let Some(ring) = log_ring() else { return };
    let mut guard = LOG_FILE.lock().unwrap();
    let Some(file) = guard.as_mut() else { return };
    let (new, pushed) = ring.since_push(file.written);
    if new.is_empty() {
        return;
    }
    let result = if file.lines + new.len() > ring.capacity() * 2 {
        // Rewrite the file from the whole ring and the push count read with
        // it, so an entry pushed meanwhile is written by the next flush, once.
        let (all, at) = ring.snapshot();
        write_lines(&file.path, &all, false).map(|_| (all.len(), at))
    } else {
        write_lines(&file.path, &new, true).map(|_| (file.lines + new.len(), pushed))
    };
    match result {
        Ok((lines, written)) => {
            file.lines = lines;
            file.written = written;
        }
        Err(e) => tracing::warn!(
            "[log] could not write the request log to {}: {e}",
            file.path.display()
        ),
    }
}

/// Load the request log file into the ring and start the writer thread.
pub fn persist_to(path: PathBuf) {
    let Some(ring) = log_ring() else { return };
    let (entries, lines) = read_file(&path, ring.capacity());
    let restored = entries.len();
    ring.restore(entries);
    *LOG_FILE.lock().unwrap() = Some(LogFile {
        path: path.clone(),
        written: ring.pushed.load(Ordering::Relaxed),
        lines,
    });
    if restored > 0 {
        tracing::info!(
            "[log] restored {restored} request log entries from {}",
            path.display()
        );
    }
    let spawned = std::thread::Builder::new()
        .name("pylon-request-log".into())
        .spawn(|| loop {
            std::thread::sleep(FLUSH_EVERY);
            flush();
        });
    if let Err(e) = spawned {
        tracing::warn!("[log] could not start the request log writer: {e}");
    }
}

pub fn log_ring() -> Option<&'static RequestLogRing> {
    LOG_RING.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ts: &str, status: u16) -> RingEntry {
        RingEntry {
            timestamp: ts.into(),
            method: "GET".into(),
            path: "/x".into(),
            status,
            cpu_ms: 1,
            bytes_in: 0,
            bytes_out: 0,
            user: None,
            studio: false,
            error: None,
        }
    }

    #[test]
    fn tail_since_returns_strictly_newer_entries_newest_first() {
        let ring = RequestLogRing::new();
        ring.push(entry("2026-05-16T10:00:00.000Z", 200));
        ring.push(entry("2026-05-16T10:00:01.000Z", 200));
        ring.push(entry("2026-05-16T10:00:02.000Z", 500));

        let out = ring.tail_since(Some("2026-05-16T10:00:00.500Z"));
        assert_eq!(out.len(), 2, "the t=0.000 entry must be excluded");
        // Newest-first ordering.
        assert_eq!(out[0].timestamp, "2026-05-16T10:00:02.000Z");
        assert_eq!(out[1].timestamp, "2026-05-16T10:00:01.000Z");
    }

    #[test]
    fn tail_since_none_returns_everything() {
        let ring = RequestLogRing::new();
        ring.push(entry("2026-05-16T10:00:00.000Z", 200));
        ring.push(entry("2026-05-16T10:00:01.000Z", 500));
        let out = ring.tail_since(None);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn tail_since_exact_match_excludes_the_cursor_entry() {
        // Strict > semantics — dashboard sets cursor = last seen
        // timestamp, and we must NOT re-ship that row on the next
        // poll. Mirrors the Tinybird pipe's `timestamp > {{since}}`.
        let ring = RequestLogRing::new();
        ring.push(entry("2026-05-16T10:00:00.000Z", 200));
        let out = ring.tail_since(Some("2026-05-16T10:00:00.000Z"));
        assert!(out.is_empty());
    }

    #[test]
    fn the_file_keeps_the_log_across_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.jsonl");
        let ring = RequestLogRing::with_capacity(100);
        ring.push(entry("2026-05-16T10:00:00.000Z", 200));
        ring.push(entry("2026-05-16T10:00:01.000Z", 500));
        let (new, pushed) = ring.since_push(0);
        write_lines(&path, &new, true).unwrap();
        assert_eq!(pushed, 2);

        // A new process reads the file back.
        let (read, lines) = read_file(&path, 100);
        assert_eq!(lines, 2);
        let next = RequestLogRing::with_capacity(100);
        next.restore(read);
        next.push(entry("2026-05-16T10:00:02.000Z", 200));
        let all = next.tail_since(None);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].timestamp, "2026-05-16T10:00:02.000Z");
        assert_eq!(all[2].timestamp, "2026-05-16T10:00:00.000Z");
        // A restored entry is not new: only the push after it is.
        assert_eq!(next.since_push(0).0.len(), 1);
    }

    #[test]
    fn a_rewrite_keeps_the_restored_entries() {
        let ring = RequestLogRing::with_capacity(100);
        ring.restore(vec![
            entry("2026-05-16T10:00:00.000Z", 200),
            entry("2026-05-16T10:00:01.000Z", 200),
        ]);
        ring.push(entry("2026-05-16T10:00:02.000Z", 200));
        let (all, pushed) = ring.snapshot();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].timestamp, "2026-05-16T10:00:00.000Z");
        assert_eq!(pushed, 1);
    }

    #[test]
    fn only_new_entries_are_written_and_a_lost_window_returns_the_ring() {
        let ring = RequestLogRing::with_capacity(100);
        for i in 0..5 {
            ring.push(entry(&format!("2026-05-16T10:00:0{i}.000Z"), 200));
        }
        let (new, pushed) = ring.since_push(3);
        assert_eq!((new.len(), pushed), (2, 5));
        assert_eq!(new[0].timestamp, "2026-05-16T10:00:03.000Z");
        // More pushes than the ring holds: the ring is what is left.
        for i in 0..150 {
            ring.push(entry(&format!("2026-05-16T11:{:02}:00.000Z", i % 60), 200));
        }
        assert_eq!(ring.since_push(0).0.len(), 100);
    }

    #[test]
    fn reading_skips_a_broken_line_and_keeps_the_last_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.jsonl");
        let mut text = String::new();
        for i in 0..5 {
            text.push_str(
                &serde_json::to_string(&entry(&format!("2026-05-16T10:00:0{i}.000Z"), 200))
                    .unwrap(),
            );
            text.push('\n');
        }
        text.push_str("{not json\n");
        std::fs::write(&path, text).unwrap();
        let (read, lines) = read_file(&path, 3);
        assert_eq!(lines, 6);
        assert_eq!(read.len(), 3);
        assert_eq!(read[2].timestamp, "2026-05-16T10:00:04.000Z");
    }

    #[test]
    fn the_size_comes_from_the_environment_within_bounds() {
        // Run in one test: the variable is process-wide.
        unsafe { std::env::remove_var("PYLON_REQUEST_LOG_SIZE") };
        assert_eq!(capacity_from_env(), RING_CAPACITY);
        unsafe { std::env::set_var("PYLON_REQUEST_LOG_SIZE", "5000") };
        assert_eq!(capacity_from_env(), 5000);
        unsafe { std::env::set_var("PYLON_REQUEST_LOG_SIZE", "5") };
        assert_eq!(capacity_from_env(), MIN_CAPACITY);
        unsafe { std::env::set_var("PYLON_REQUEST_LOG_SIZE", "10000000") };
        assert_eq!(capacity_from_env(), MAX_CAPACITY);
        unsafe { std::env::remove_var("PYLON_REQUEST_LOG_SIZE") };
    }

    #[test]
    fn capacity_drops_oldest_first() {
        let ring = RequestLogRing::new();
        for i in 0..(RING_CAPACITY + 50) {
            ring.push(entry(&format!("2026-05-16T10:00:{:02}.000Z", i % 60), 200));
        }
        let all = ring.tail_since(None);
        assert_eq!(all.len(), RING_CAPACITY);
    }
}
