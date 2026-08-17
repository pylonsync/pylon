//! Resumable function streams.
//!
//! Every SSE function call (`POST /api/fn/:name` with an
//! `Accept: text/event-stream` that actually streams) gets a buffered,
//! sequence-numbered stream in this hub. The producer (the Bun handler's
//! `ctx.stream.write`) appends frames here and NEVER blocks on the HTTP
//! transport; HTTP connections are just subscribers that replay from a
//! cursor and then live-tail. A client that disconnects mid-stream —
//! closed laptop, flaky mobile network, proxy idle timeout — reconnects
//! to `GET /api/fn-streams/<id>` with its last seen `id:` and catches
//! up. The terminal `result`/`error` frame is buffered too, so a client
//! that comes back after the handler finished still gets the answer.
//!
//! Scope, honestly: buffers are in-memory and bounded. Resume survives
//! any transport failure; it does not survive a server restart — the
//! producing handler wouldn't survive one either. Restart-durable
//! execution is what `ctx.workflows` is for.
//!
//! Model: per-stream ring under a per-stream mutex + condvar
//! (`Arc<StreamEntry>`), with the hub map itself under a coarse mutex
//! only for id lookup. Readers PULL frames from the ring (lossless, no
//! per-subscriber queues to overflow); appends `notify_all` waiters.
//! Byte-cap eviction drops the oldest frames and advances a truncation
//! watermark — a resume below the watermark gets `Gone`, mirroring the
//! sync change-log's 410 contract.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Default per-stream buffer cap (bytes of frame data). At ~4 bytes per
/// streamed token this holds several full LLM responses.
const DEFAULT_MAX_BYTES_PER_STREAM: usize = 4 * 1024 * 1024;
/// Default retention for completed streams.
const DEFAULT_RETAIN_SECS: u64 = 3600;
/// Default cap on concurrently buffered streams. Live SSE connections
/// are separately bounded by the StreamLimiter; this bounds memory held
/// for disconnected/completed streams.
const DEFAULT_MAX_STREAMS: usize = 2000;
/// A Live stream that has produced nothing and been read by nobody for
/// this long is considered abandoned (its call long since hit the fn
/// timeout) and is reaped by cleanup.
const ABANDONED_LIVE_SECS: u64 = 24 * 3600;
/// Per-frame bookkeeping cost added to the byte accounting: seq +
/// Option<String> event + String headers + VecDeque slot. Without
/// this, a handler writing 1-byte chunks would pin ~100x its nominal
/// budget in real memory.
const FRAME_OVERHEAD_BYTES: usize = 96;
/// Default global budget across ALL buffered streams. The per-stream
/// cap bounds one stream; this bounds the fleet — completed streams
/// evict oldest-first when the total crosses it, and new streams are
/// refused rather than letting buffers OOM the machine.
const DEFAULT_MAX_TOTAL_BYTES: usize = 256 * 1024 * 1024;
/// A completed stream younger than this is protected from cap/budget
/// eviction — "come back for the result" must survive an attacker
/// churning new streams.
const EVICTION_GRACE_SECS: u64 = 300;

fn frame_cost(event: Option<&str>, data: &str) -> usize {
    data.len() + event.map(str::len).unwrap_or(0) + FRAME_OVERHEAD_BYTES
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFrame {
    pub seq: u64,
    /// SSE event type (`ctx.stream.writeEvent`); `None` for plain data
    /// frames. Terminal frames use `"result"` / `"error"`.
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamStatus {
    Live,
    Done,
    Failed,
}

/// Ownership snapshot for the resume endpoint's auth check.
#[derive(Debug, Clone)]
pub struct StreamMeta {
    pub fn_name: String,
    pub user_id: Option<String>,
    pub tenant_id: Option<String>,
    pub status: StreamStatus,
}

struct StreamBuf {
    frames: std::collections::VecDeque<StreamFrame>,
    /// Next seq to assign. Seqs start at 1 (0 = "from the beginning").
    next_seq: u64,
    /// Lowest seq still in the ring. A resume cursor below
    /// `oldest_seq - 1` cannot be served contiguously → Gone.
    oldest_seq: u64,
    bytes: usize,
    status: StreamStatus,
    meta: StreamMeta,
    created_at: Instant,
    completed_at: Option<Instant>,
    /// Last time a reader pulled frames — keeps an abandoned-stream
    /// reap from killing a stream someone is actively tailing.
    last_read_at: Instant,
}

pub struct StreamEntry {
    state: Mutex<StreamBuf>,
    cond: Condvar,
}

/// Result of one blocking wait for frames past a cursor.
#[derive(Debug)]
pub enum WaitOutcome {
    /// Frames after the cursor, in order. May include the terminal
    /// frame; check `ended`.
    Frames(Vec<StreamFrame>, /* ended: */ bool),
    /// Timed out with nothing new and the stream still live — emit a
    /// heartbeat and wait again.
    Heartbeat,
    /// Stream is terminal and the cursor is at the tip — nothing more
    /// will come.
    Ended,
    /// The cursor points below the retention window (byte-cap eviction
    /// dropped those frames). Carries the oldest seq still served.
    Gone(u64),
}

pub struct StreamHub {
    streams: Mutex<HashMap<String, Arc<StreamEntry>>>,
    max_bytes_per_stream: usize,
    retain: Duration,
    max_streams: usize,
    /// Sum of every stream's accounted bytes. Updated under entry
    /// locks; read without.
    total_bytes: std::sync::atomic::AtomicUsize,
    max_total_bytes: usize,
    eviction_grace: Duration,
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|s| s.trim().parse().ok())
}

impl StreamHub {
    pub fn from_env() -> Self {
        Self {
            streams: Mutex::new(HashMap::new()),
            max_bytes_per_stream: env_parse::<usize>("PYLON_STREAM_BUFFER_MAX_BYTES")
                .filter(|n| *n > 0)
                .unwrap_or(DEFAULT_MAX_BYTES_PER_STREAM),
            retain: Duration::from_secs(
                env_parse::<u64>("PYLON_STREAM_RETAIN_SECS")
                    .filter(|n| *n > 0)
                    .unwrap_or(DEFAULT_RETAIN_SECS),
            ),
            max_streams: env_parse::<usize>("PYLON_STREAM_MAX")
                .filter(|n| *n > 0)
                .unwrap_or(DEFAULT_MAX_STREAMS),
            total_bytes: std::sync::atomic::AtomicUsize::new(0),
            max_total_bytes: env_parse::<usize>("PYLON_STREAM_TOTAL_MAX_BYTES")
                .filter(|n| *n > 0)
                .unwrap_or(DEFAULT_MAX_TOTAL_BYTES),
            eviction_grace: Duration::from_secs(EVICTION_GRACE_SECS),
        }
    }

    #[cfg(test)]
    pub fn with_caps(max_bytes_per_stream: usize, retain_secs: u64, max_streams: usize) -> Self {
        Self {
            streams: Mutex::new(HashMap::new()),
            max_bytes_per_stream,
            retain: Duration::from_secs(retain_secs),
            max_streams,
            total_bytes: std::sync::atomic::AtomicUsize::new(0),
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            eviction_grace: Duration::from_secs(EVICTION_GRACE_SECS),
        }
    }

    #[cfg(test)]
    pub fn set_eviction_grace_for_test(&mut self, secs: u64) {
        self.eviction_grace = Duration::from_secs(secs);
    }

    /// Register a new stream. Returns `None` when the hub is at its
    /// stream cap and nothing is evictable — the caller then serves the
    /// call unbuffered (streams, but can't resume) rather than failing.
    pub fn create(
        &self,
        stream_id: String,
        fn_name: &str,
        user_id: Option<&str>,
        tenant_id: Option<&str>,
    ) -> Option<Arc<StreamEntry>> {
        use std::sync::atomic::Ordering;
        // Refuse new buffers when the fleet-wide byte budget is spent —
        // the caller answers 503 rather than this hub OOMing the box.
        if self.total_bytes.load(Ordering::Relaxed) >= self.max_total_bytes {
            self.enforce_global_budget();
            if self.total_bytes.load(Ordering::Relaxed) >= self.max_total_bytes {
                return None;
            }
        }
        let mut streams = self.streams.lock().unwrap();
        if streams.len() >= self.max_streams {
            // Evict the oldest terminal stream to make room — but only
            // past the eviction grace window. A fresh completed stream
            // holds someone's un-fetched final result; refusing the NEW
            // stream beats destroying an existing one (an attacker
            // churning streams must not be able to evict other users'
            // results).
            let now = Instant::now();
            let victim = streams
                .iter()
                .filter_map(|(id, e)| {
                    let st = e.state.lock().unwrap();
                    match (st.status, st.completed_at) {
                        (StreamStatus::Live, _) | (_, None) => None,
                        (_, Some(done)) if now.duration_since(done) >= self.eviction_grace => {
                            Some((id.clone(), done, st.bytes))
                        }
                        _ => None,
                    }
                })
                .min_by_key(|(_, at, _)| *at);
            match victim {
                Some((id, _, bytes)) => {
                    streams.remove(&id);
                    self.total_bytes.fetch_sub(bytes, Ordering::Relaxed);
                }
                None => return None,
            }
        }
        let now = Instant::now();
        let entry = Arc::new(StreamEntry {
            state: Mutex::new(StreamBuf {
                frames: std::collections::VecDeque::new(),
                next_seq: 1,
                oldest_seq: 1,
                bytes: 0,
                status: StreamStatus::Live,
                meta: StreamMeta {
                    fn_name: fn_name.to_string(),
                    user_id: user_id.map(str::to_string),
                    tenant_id: tenant_id.map(str::to_string),
                    status: StreamStatus::Live,
                },
                created_at: now,
                completed_at: None,
                last_read_at: now,
            }),
            cond: Condvar::new(),
        });
        streams.insert(stream_id, Arc::clone(&entry));
        Some(entry)
    }

    pub fn get(&self, stream_id: &str) -> Option<Arc<StreamEntry>> {
        self.streams.lock().unwrap().get(stream_id).cloned()
    }

    /// Drop a stream immediately. Used when a call turns out not to
    /// stream at all (plain-JSON answer) — nothing to resume.
    pub fn remove(&self, stream_id: &str) {
        use std::sync::atomic::Ordering;
        if let Some(e) = self.streams.lock().unwrap().remove(stream_id) {
            let bytes = e.state.lock().unwrap().bytes;
            self.total_bytes.fetch_sub(bytes, Ordering::Relaxed);
        }
    }

    /// Evict whole terminal streams (oldest-completed first, outside
    /// the grace window) until the global byte budget is met. Called
    /// after appends when the fleet total crosses the budget; never
    /// touches Live streams. Lock order map→entry, same as create.
    pub fn enforce_global_budget(&self) {
        use std::sync::atomic::Ordering;
        if self.total_bytes.load(Ordering::Relaxed) < self.max_total_bytes {
            return;
        }
        let mut streams = self.streams.lock().unwrap();
        let now = Instant::now();
        let mut candidates: Vec<(String, Instant, usize)> = streams
            .iter()
            .filter_map(|(id, e)| {
                let st = e.state.lock().unwrap();
                match (st.status, st.completed_at) {
                    (StreamStatus::Live, _) | (_, None) => None,
                    (_, Some(done)) if now.duration_since(done) >= self.eviction_grace => {
                        Some((id.clone(), done, st.bytes))
                    }
                    _ => None,
                }
            })
            .collect();
        candidates.sort_by_key(|(_, at, _)| *at);
        for (id, _, bytes) in candidates {
            if self.total_bytes.load(Ordering::Relaxed) < self.max_total_bytes {
                break;
            }
            streams.remove(&id);
            self.total_bytes.fetch_sub(bytes, Ordering::Relaxed);
        }
    }

    /// Drop completed streams past retention, and Live streams that
    /// have been abandoned (no reads, no writes) for 24h. Returns the
    /// number removed.
    pub fn cleanup_expired(&self) -> usize {
        use std::sync::atomic::Ordering;
        let now = Instant::now();
        let mut streams = self.streams.lock().unwrap();
        let before = streams.len();
        let mut freed = 0usize;
        streams.retain(|_, e| {
            let st = e.state.lock().unwrap();
            let keep = match st.status {
                StreamStatus::Live => {
                    let last_activity = st.last_read_at.max(st.created_at);
                    now.duration_since(last_activity) < Duration::from_secs(ABANDONED_LIVE_SECS)
                }
                _ => match st.completed_at {
                    Some(done) => now.duration_since(done) < self.retain,
                    None => true,
                },
            };
            if !keep {
                freed += st.bytes;
            }
            keep
        });
        if freed > 0 {
            self.total_bytes.fetch_sub(freed, Ordering::Relaxed);
        }
        before - streams.len()
    }

    /// Buffered-stream count (metrics / tests).
    pub fn len(&self) -> usize {
        self.streams.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Append a data frame. Returns the assigned seq. Callers should
    /// follow up with [`Self::enforce_global_budget`] outside any entry
    /// lock when the append pushed the fleet total over budget.
    pub fn append(&self, entry: &StreamEntry, event: Option<&str>, data: &str) -> u64 {
        use std::sync::atomic::Ordering;
        let cost = frame_cost(event, data);
        let mut st = entry.state.lock().unwrap();
        let seq = st.next_seq;
        st.next_seq += 1;
        st.bytes += cost;
        self.total_bytes.fetch_add(cost, Ordering::Relaxed);
        st.frames.push_back(StreamFrame {
            seq,
            event: event.map(str::to_string),
            data: data.to_string(),
        });
        self.evict_over_cap(&mut st);
        drop(st);
        entry.cond.notify_all();
        seq
    }

    /// Byte-cap eviction: drop oldest, advance the truncation
    /// watermark. Never evicts the newest frame.
    fn evict_over_cap(&self, st: &mut StreamBuf) {
        use std::sync::atomic::Ordering;
        while st.bytes > self.max_bytes_per_stream && st.frames.len() > 1 {
            if let Some(dropped) = st.frames.pop_front() {
                let cost = frame_cost(dropped.event.as_deref(), &dropped.data);
                st.bytes -= cost;
                self.total_bytes.fetch_sub(cost, Ordering::Relaxed);
                st.oldest_seq = dropped.seq + 1;
            }
        }
    }

    /// Terminal success. The result frame is buffered like any other so
    /// late resumes still receive it.
    pub fn complete(&self, entry: &StreamEntry, result_json: &str) {
        self.finish(entry, StreamStatus::Done, "result", result_json);
    }

    /// Terminal failure.
    pub fn fail(&self, entry: &StreamEntry, error_json: &str) {
        self.finish(entry, StreamStatus::Failed, "error", error_json);
    }

    fn finish(&self, entry: &StreamEntry, status: StreamStatus, event: &str, data: &str) {
        use std::sync::atomic::Ordering;
        let cost = frame_cost(Some(event), data);
        let mut st = entry.state.lock().unwrap();
        if st.status != StreamStatus::Live {
            return; // already terminal — keep the first outcome
        }
        let seq = st.next_seq;
        st.next_seq += 1;
        st.bytes += cost;
        self.total_bytes.fetch_add(cost, Ordering::Relaxed);
        st.frames.push_back(StreamFrame {
            seq,
            event: Some(event.to_string()),
            data: data.to_string(),
        });
        // A huge return value must not bypass the per-stream cap; the
        // terminal frame itself is the back of the deque and survives.
        self.evict_over_cap(&mut st);
        st.status = status;
        st.meta.status = status;
        st.completed_at = Some(Instant::now());
        drop(st);
        entry.cond.notify_all();
    }

    pub fn meta(&self, entry: &StreamEntry) -> StreamMeta {
        entry.state.lock().unwrap().meta.clone()
    }

    /// Lowest seq still served from the ring. A resume cursor must be
    /// >= this - 1 to be served contiguously; below it → 410.
    pub fn oldest_seq(&self, entry: &StreamEntry) -> u64 {
        entry.state.lock().unwrap().oldest_seq
    }

    /// Blocking wait for frames past `after_seq`. `timeout` bounds the
    /// wait so the caller can interleave heartbeats and notice dead
    /// sockets. Lossless: frames come from the ring, not a queue.
    pub fn wait_frames(
        &self,
        entry: &StreamEntry,
        after_seq: u64,
        timeout: Duration,
    ) -> WaitOutcome {
        let mut st = entry.state.lock().unwrap();
        st.last_read_at = Instant::now();
        loop {
            // Cursor below the retention window? (after_seq must be
            // >= oldest_seq - 1 to serve contiguously from the ring.)
            if after_seq + 1 < st.oldest_seq {
                return WaitOutcome::Gone(st.oldest_seq);
            }
            let fresh: Vec<StreamFrame> = st
                .frames
                .iter()
                .filter(|f| f.seq > after_seq)
                .cloned()
                .collect();
            if !fresh.is_empty() {
                let ended = st.status != StreamStatus::Live;
                let at_tip = fresh.last().map(|f| f.seq + 1) == Some(st.next_seq);
                return WaitOutcome::Frames(fresh, ended && at_tip);
            }
            if st.status != StreamStatus::Live {
                return WaitOutcome::Ended;
            }
            let (guard, res) = entry.cond.wait_timeout(st, timeout).unwrap();
            st = guard;
            st.last_read_at = Instant::now();
            if res.timed_out() {
                // Re-check once before signalling heartbeat: the wait
                // can time out concurrently with an append.
                let has_fresh = st.frames.iter().any(|f| f.seq > after_seq);
                let live = st.status == StreamStatus::Live;
                if !has_fresh {
                    if live {
                        return WaitOutcome::Heartbeat;
                    }
                    return WaitOutcome::Ended;
                }
                // fall through to the collect at loop top
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> StreamHub {
        StreamHub::with_caps(1024, 3600, 10)
    }

    fn frames(o: WaitOutcome) -> (Vec<StreamFrame>, bool) {
        match o {
            WaitOutcome::Frames(f, ended) => (f, ended),
            other => panic!("expected frames, got {other:?}"),
        }
    }

    #[test]
    fn append_replay_and_terminal() {
        let h = hub();
        let e = h.create("s1".into(), "chat", Some("u1"), None).unwrap();
        assert_eq!(h.append(&e, None, "hello"), 1);
        assert_eq!(h.append(&e, Some("tick"), "x"), 2);
        h.complete(&e, "{\"ok\":true}");

        // Full replay from 0.
        let (f, ended) = frames(h.wait_frames(&e, 0, Duration::from_millis(10)));
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].data, "hello");
        assert_eq!(f[1].event.as_deref(), Some("tick"));
        assert_eq!(f[2].event.as_deref(), Some("result"));
        assert!(ended);

        // Mid-cursor resume.
        let (f, ended) = frames(h.wait_frames(&e, 1, Duration::from_millis(10)));
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].seq, 2);
        assert!(ended);

        // At-tip resume on a terminal stream.
        assert!(matches!(
            h.wait_frames(&e, 3, Duration::from_millis(10)),
            WaitOutcome::Ended
        ));

        // A second terminal is ignored — first outcome wins.
        h.fail(&e, "{\"code\":\"X\"}");
        let (f, _) = frames(h.wait_frames(&e, 0, Duration::from_millis(10)));
        assert_eq!(f.len(), 3);
        assert_eq!(h.meta(&e).status, StreamStatus::Done);
    }

    #[test]
    fn live_wait_heartbeats_then_delivers() {
        let h = Arc::new(hub());
        let e = h.create("s1".into(), "chat", None, None).unwrap();
        assert!(matches!(
            h.wait_frames(&e, 0, Duration::from_millis(20)),
            WaitOutcome::Heartbeat
        ));
        let h2 = Arc::clone(&h);
        let e2 = h.get("s1").unwrap();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            h2.append(&e2, None, "late");
        });
        let (f, ended) = frames(h.wait_frames(&e, 0, Duration::from_secs(5)));
        assert_eq!(f[0].data, "late");
        assert!(!ended);
        t.join().unwrap();
    }

    #[test]
    fn byte_cap_evicts_oldest_and_gones_stale_cursors() {
        let h = StreamHub::with_caps(100, 3600, 10);
        let e = h.create("s1".into(), "chat", None, None).unwrap();
        // 10 frames × 30 bytes overflows the 100-byte cap.
        for i in 0..10 {
            h.append(&e, None, &format!("{:030}", i));
        }
        // Cursor 0 fell off retention.
        match h.wait_frames(&e, 0, Duration::from_millis(10)) {
            WaitOutcome::Gone(oldest) => assert!(oldest > 1),
            other => panic!("expected Gone, got {other:?}"),
        }
        // A recent cursor still works.
        let (f, _) = frames(h.wait_frames(&e, 9, Duration::from_millis(10)));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].seq, 10);
    }

    #[test]
    fn stream_cap_protects_fresh_results_then_evicts_stale_ones() {
        let mut h = StreamHub::with_caps(1024, 3600, 2);
        h.set_eviction_grace_for_test(3600);
        let a = h.create("a".into(), "f", None, None).unwrap();
        let _b = h.create("b".into(), "f", None, None).unwrap();
        // Hub full of LIVE streams — refuse (caller answers 503).
        assert!(h.create("c".into(), "f", None, None).is_none());
        // A FRESH completed stream is inside the eviction grace window:
        // its un-fetched result must survive an attacker churning new
        // streams, so create still refuses.
        h.complete(&a, "null");
        assert!(
            h.create("c".into(), "f", None, None).is_none(),
            "fresh completed streams are protected from eviction"
        );
        assert!(h.get("a").is_some());
        // Outside the grace window the stale completed stream is fair game.
        h.set_eviction_grace_for_test(0);
        std::thread::sleep(Duration::from_millis(5));
        assert!(h.create("c".into(), "f", None, None).is_some());
        assert!(h.get("a").is_none(), "stale completed stream was evicted");
        assert!(h.get("b").is_some());
    }

    #[test]
    fn byte_accounting_tracks_total_and_frees_on_removal() {
        use std::sync::atomic::Ordering;
        let h = StreamHub::with_caps(1024 * 1024, 3600, 10);
        let e = h.create("s1".into(), "f", None, None).unwrap();
        h.append(&e, None, "0123456789");
        h.append(&e, Some("tick"), "x");
        let total = h.total_bytes.load(Ordering::Relaxed);
        // data + event bytes + per-frame overhead, both frames.
        assert_eq!(
            total,
            10 + FRAME_OVERHEAD_BYTES + 1 + 4 + FRAME_OVERHEAD_BYTES
        );
        h.complete(&e, "null");
        let with_terminal = h.total_bytes.load(Ordering::Relaxed);
        assert!(with_terminal > total);
        h.remove("s1");
        assert_eq!(
            h.total_bytes.load(Ordering::Relaxed),
            0,
            "removal frees the budget"
        );
    }

    #[test]
    fn cleanup_reaps_expired_completed_streams() {
        let h = StreamHub::with_caps(1024, 0, 10); // 0s retention
        let e = h.create("s1".into(), "f", None, None).unwrap();
        h.complete(&e, "null");
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(h.cleanup_expired(), 1);
        assert!(h.get("s1").is_none());
    }

    #[test]
    fn ownership_metadata_survives() {
        let h = hub();
        let e = h
            .create("s1".into(), "chat", Some("u9"), Some("t3"))
            .unwrap();
        let m = h.meta(&e);
        assert_eq!(m.fn_name, "chat");
        assert_eq!(m.user_id.as_deref(), Some("u9"));
        assert_eq!(m.tenant_id.as_deref(), Some("t3"));
        assert_eq!(m.status, StreamStatus::Live);
    }
}
