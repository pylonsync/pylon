//! Persisted SQLite change-log seq allocator.
//!
//! This is the third attempt at solving the "client cursor outlives
//! server restart → 410 RESYNC_REQUIRED storm" problem. The earlier
//! shapes:
//!
//! - **v0.3.217 and prior** — in-memory atomic only, resets to 0 on
//!   restart. Every redeploy of chat-api caused a 410 wave (recoverable
//!   via reset+repull but visibly disruptive).
//!
//! - **v0.3.218–0.3.221** — synchronous SQLite UPDATE inside the seq
//!   provider that the mutation pipeline already holds `write_conn`
//!   for. Recursive mutex acquisition → deadlock. Symptom on chat-api:
//!   `markChannelRead` hung forever; only reads succeeded. Shipped + reverted.
//!
//! - **v0.3.223 (this)** — high-water-mark reservation done OFF the
//!   mutation hot path by a background thread with its own conn.
//!
//! ## Design
//!
//! - **In-memory atomic `current`** — the next seq to issue (minus 1).
//!   Provider closure returns `current.fetch_add(1) + 1`. No SQLite,
//!   no lock, no deadlock.
//!
//! - **Persisted high-water mark** — `_pylon_change_seq.value` on
//!   disk. Reserved in chunks of `CHUNK_SIZE` at a time. Reservation
//!   means: writing `(prev_value + CHUNK_SIZE)` to disk, atomically
//!   via `UPDATE ... SET value = value + ?`. After the write, seqs
//!   in `(prev_value, prev_value + CHUNK_SIZE]` are safe to issue
//!   without further disk activity.
//!
//! - **Background reservation thread** — owns its own
//!   `Arc<Runtime>` for SQLite access. Receives extension requests
//!   via mpsc channel from the provider closure. When `current`
//!   approaches `reservation_end`, the closure sends a request; the
//!   bg thread runs the UPDATE on its own write_conn lock acquisition
//!   (which queues behind any active mutation tx — no recursive lock).
//!
//! ## Crash safety
//!
//! After ANY crash, the disk's persisted value is `>=` every seq we
//! ever issued. Proof: we always write `(end + CHUNK_SIZE)` to disk
//! BEFORE any seq in that new range gets returned. So persisted is
//! always at least one CHUNK_SIZE ahead of `current`. On next boot
//! we read this value and reserve fresh chunks above it. Client
//! cursors saved before the crash are <= persisted, so the next
//! `pull` returns events normally instead of 410'ing.
//!
//! ## Overshoot
//!
//! If a sustained mutation burst issues seqs faster than the bg
//! thread can extend the reservation, `current` could (in theory)
//! overshoot `reservation_end`. To avoid this becoming a
//! crash-loses-seqs bug, `next()` blocks (yielding) once `current`
//! reaches the reservation. The blocking path is per-call cheap and
//! only triggers under sustained mutation rates > ~10k/sec, which is
//! well above any current pylon workload. In practice it's never
//! seen.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use crate::Runtime;

/// Default chunk size — the amount of seqs reserved on disk per extension.
/// 1000 means: at steady-state, one SQLite UPDATE per 1000 mutations.
const DEFAULT_CHUNK_SIZE: u64 = 1000;

/// When the remaining unused reservation drops below `chunk_size / EXTEND_TRIGGER_DIV`,
/// the provider closure sends an extension request to the bg thread.
/// Set generously low so requests fire while the buffer is still healthy.
const EXTEND_TRIGGER_DIV: u64 = 5;

/// Allocates monotonically-increasing seqs backed by a persisted
/// high-water mark on a SQLite `_pylon_change_seq` table. See module
/// docstring for the full design.
pub struct SqliteSeqAllocator {
    /// The last seq issued. Provider returns `fetch_add(1) + 1` so the
    /// first issued seq is `initial + 1`.
    current: Arc<AtomicU64>,
    /// The furthest seq the bg thread has reserved on disk. While
    /// `current <= reservation_end`, no disk write is needed.
    reservation_end: Arc<AtomicU64>,
    /// Sends extension requests to the bg thread. Each `u64` is the
    /// new desired reservation_end (caller-computed). Bg thread CAS's
    /// the atomic if the request is still relevant.
    persist_sender: mpsc::Sender<ExtendRequest>,
    /// Chunk size copied from constructor — used by `next()` to decide
    /// when to request more.
    chunk_size: u64,
    /// The seq floor for this boot: the value the FIRST `next()` issues
    /// one above (`floor + 1`). Equals the persisted high-water that
    /// existed BEFORE this allocator reserved its first chunk. The
    /// ChangeLog must be seeded with `with_initial_seq(floor)` so its
    /// snapshot cursor sits at — not above — the first seq we'll issue;
    /// otherwise every event in the first chunk lands below the cursor
    /// and clients silently drop the deltas.
    floor: u64,
}

enum ExtendRequest {
    /// Persist a new reservation_end. Bg thread runs UPDATE.
    Extend(u64),
    /// Final shutdown signal — bg thread exits.
    Shutdown,
}

impl SqliteSeqAllocator {
    /// Boot-time construction. Reads the current persisted value
    /// (created by `Runtime::bootstrap_sqlite_change_seq`), reserves
    /// the first chunk synchronously (so no seq is ever issued
    /// without disk backing), and spawns the bg reservation thread.
    pub fn new(runtime: Arc<Runtime>) -> Option<Self> {
        Self::new_with_chunk(runtime, DEFAULT_CHUNK_SIZE)
    }

    /// Same as `new` but with a configurable chunk size. Tests use
    /// small chunks (e.g. 5) to exercise the extension path quickly.
    pub fn new_with_chunk(runtime: Arc<Runtime>, chunk_size: u64) -> Option<Self> {
        // Reserve the first chunk synchronously. This both:
        //   1. Confirms the SQLite table is reachable + writable
        //      (failure here surfaces at boot, not on first
        //      mutation).
        //   2. Establishes the initial reservation_end above any
        //      previously-persisted high water.
        let initial_end = runtime.reserve_sqlite_change_seq(chunk_size)?;
        // The seqs we own this boot are (initial_end - chunk_size,
        // initial_end]. Start `current` at the bottom of that range
        // so fetch_add(1) yields the first seq.
        let initial_current = initial_end - chunk_size;

        let current = Arc::new(AtomicU64::new(initial_current));
        let reservation_end = Arc::new(AtomicU64::new(initial_end));

        let (tx, rx) = mpsc::channel::<ExtendRequest>();

        // Bg thread. Owns an Arc<Runtime> + the atomic. Receives
        // extension requests; CASes the atomic to publish the new
        // ceiling. The CAS uses the OLD reservation_end (from the
        // request) as the expected value — if it doesn't match, the
        // request is stale (multiple requests coalesced) and we
        // skip. This bounds the number of disk writes under burst.
        let rt_bg = Arc::clone(&runtime);
        let res_end_bg = Arc::clone(&reservation_end);
        thread::Builder::new()
            .name("pylon-seq-persister".into())
            .stack_size(128 * 1024)
            .spawn(move || loop {
                match rx.recv() {
                    Ok(ExtendRequest::Extend(target)) => {
                        // Only persist if `target` would actually
                        // advance the high water. Stale requests
                        // (others raced ahead) are no-ops.
                        let cur_end = res_end_bg.load(Ordering::Acquire);
                        if target <= cur_end {
                            continue;
                        }
                        // Bump the persisted value by enough to
                        // reach `target`. UPDATE blocks until any
                        // active mutation tx commits — no recursive
                        // lock because this is a different thread
                        // from the mutation pipeline.
                        let delta = target - cur_end;
                        if let Some(new_end) = rt_bg.reserve_sqlite_change_seq(delta) {
                            res_end_bg.store(new_end, Ordering::Release);
                        }
                        // Failure: log + drop the request. The
                        // existing reservation is still valid, so
                        // we just keep using it until the next
                        // trigger.
                    }
                    Ok(ExtendRequest::Shutdown) | Err(_) => return,
                }
            })
            .ok()?;

        Some(Self {
            current,
            reservation_end,
            persist_sender: tx,
            chunk_size,
            floor: initial_current,
        })
    }

    /// The seq floor for this boot — the first `next()` returns
    /// `floor_seq() + 1`. Callers wire the ChangeLog with
    /// `with_initial_seq(allocator.floor_seq())` so the snapshot cursor
    /// matches the seqs this allocator issues. Reading the persisted
    /// `_pylon_change_seq` value instead would capture the
    /// POST-reservation high-water (floor + chunk_size), seeding the
    /// cursor a whole chunk too high and dropping every delta below it.
    pub fn floor_seq(&self) -> u64 {
        self.floor
    }

    /// Issue the next seq. Hot-path safe: in-memory atomic only;
    /// no SQLite, no lock. Sends a fire-and-forget extension
    /// request to the bg thread when the remaining reservation
    /// dips below the trigger threshold.
    ///
    /// If a sustained burst overruns the bg thread's ability to
    /// extend (current exceeds reservation_end), this method yields
    /// briefly until the bg thread catches up. The wait is bounded
    /// by the bg thread's SQLite write latency (~ms under normal
    /// load).
    pub fn next(&self) -> u64 {
        loop {
            let issued = self.current.fetch_add(1, Ordering::AcqRel) + 1;
            let end = self.reservation_end.load(Ordering::Acquire);
            let remaining = end.saturating_sub(issued);
            // Request an extension when we're 80% through the chunk
            // (`< chunk_size / EXTEND_TRIGGER_DIV` left). The send is
            // fire-and-forget; if the channel is closed (shutdown
            // race) we silently drop.
            if remaining < self.chunk_size / EXTEND_TRIGGER_DIV {
                let target = end + self.chunk_size;
                let _ = self.persist_sender.send(ExtendRequest::Extend(target));
            }
            if issued <= end {
                return issued;
            }
            // Overshoot — we already fetched_add'd past the
            // reservation. Roll back by re-decrementing (so the
            // next call retries the same slot) and yield to let the
            // bg thread catch up. Realistically: only triggers
            // under sustained > 10k mutations/sec.
            //
            // Note: roll-back via fetch_sub is safe because we're
            // the only mutator; the bg thread doesn't touch
            // `current`. Other allocators racing on the same atomic
            // will simply observe one lower starting point on their
            // retry — equivalent to having lost the race.
            self.current.fetch_sub(1, Ordering::AcqRel);
            thread::sleep(Duration::from_micros(100));
        }
    }

    /// Snapshot of the highest seq successfully reserved on disk.
    /// Used by tests + diagnostics; not the hot path.
    #[cfg(test)]
    pub fn reservation_end(&self) -> u64 {
        self.reservation_end.load(Ordering::Acquire)
    }

    /// Snapshot of the current in-memory issued counter. The next
    /// `next()` call returns `current + 1`. Used by tests +
    /// diagnostics.
    #[cfg(test)]
    pub fn current(&self) -> u64 {
        self.current.load(Ordering::Acquire)
    }
}

impl Drop for SqliteSeqAllocator {
    fn drop(&mut self) {
        // Best-effort: tell the bg thread to exit. If the channel is
        // already closed (e.g., panic earlier), the send fails
        // silently. The thread exits on its own when the receiver
        // drops anyway.
        let _ = self.persist_sender.send(ExtendRequest::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn open_runtime(tmp: &PathBuf) -> Arc<Runtime> {
        let _ = std::fs::remove_file(tmp);
        let rt = Runtime::open(tmp.to_str().unwrap(), test_manifest_minimal()).unwrap();
        rt.bootstrap_sqlite_change_seq().unwrap();
        Arc::new(rt)
    }

    fn test_manifest_minimal() -> pylon_kernel::AppManifest {
        use pylon_kernel::*;
        AppManifest {
            manifest_version: 1,
            name: "t".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
                name: "T".into(),
                fields: vec![ManifestField {
                    name: "x".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                }],
                indexes: vec![],
                relations: vec![],
                search: None,
                crdt: true,
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
        }
    }

    #[test]
    fn issues_monotonic_seqs_within_chunk() {
        let tmp = std::env::temp_dir().join("pylon-seq-mono.sqlite");
        let rt = open_runtime(&tmp);
        let alloc = SqliteSeqAllocator::new_with_chunk(rt.clone(), 100).unwrap();

        let mut last = 0;
        for _ in 0..50 {
            let s = alloc.next();
            assert!(s > last, "seq must be monotonically increasing");
            last = s;
        }
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn extends_reservation_when_approaching_boundary() {
        let tmp = std::env::temp_dir().join("pylon-seq-extend.sqlite");
        let rt = open_runtime(&tmp);
        let alloc = SqliteSeqAllocator::new_with_chunk(rt.clone(), 10).unwrap();
        let initial_end = alloc.reservation_end();
        // Issue past the 80% trigger point.
        for _ in 0..9 {
            alloc.next();
        }
        // Bg thread should extend within a moment.
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        while std::time::Instant::now() < deadline && alloc.reservation_end() == initial_end {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            alloc.reservation_end() > initial_end,
            "reservation should have extended by now (initial={initial_end}, current_end={})",
            alloc.reservation_end()
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn seqs_monotonic_across_restart() {
        let tmp = std::env::temp_dir().join("pylon-seq-restart.sqlite");
        let highest_before;
        {
            let rt = open_runtime(&tmp);
            let alloc = SqliteSeqAllocator::new_with_chunk(rt.clone(), 50).unwrap();
            for _ in 0..40 {
                alloc.next();
            }
            highest_before = alloc.current();
            // Drop alloc → bg thread exits → SqliteSeqAllocator gone.
            drop(alloc);
            // Settle: give bg thread a moment to process any
            // outstanding Extend request before we drop the runtime.
            std::thread::sleep(Duration::from_millis(50));
        }
        // Reopen runtime + allocator. New allocator must issue
        // seqs strictly greater than what the previous one issued
        // (proves crash-safe monotonicity).
        let rt = Arc::new(Runtime::open(tmp.to_str().unwrap(), test_manifest_minimal()).unwrap());
        rt.bootstrap_sqlite_change_seq().unwrap();
        let alloc2 = SqliteSeqAllocator::new_with_chunk(rt.clone(), 50).unwrap();
        let first_after = alloc2.next();
        assert!(
            first_after > highest_before,
            "post-restart first seq ({first_after}) must be > pre-restart highest ({highest_before})"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn floor_seq_is_exactly_one_below_the_first_issued_seq() {
        // The contract callers rely on: seed the ChangeLog snapshot
        // cursor with `floor_seq()` and the first delta (`next()`) lands
        // exactly one above it. If this drifts, deltas in the first
        // chunk fall at/below the cursor and clients drop them.
        let tmp = std::env::temp_dir().join("pylon-seq-floor.sqlite");
        let rt = open_runtime(&tmp);
        let alloc = SqliteSeqAllocator::new_with_chunk(rt.clone(), 100).unwrap();
        let floor = alloc.floor_seq();
        assert_eq!(
            alloc.next(),
            floor + 1,
            "first issued seq must be exactly floor_seq() + 1"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn floor_seeded_changelog_delivers_deltas_above_the_snapshot_cursor() {
        // Regression for the SQLite "live queries never update" bug: the
        // change-log snapshot cursor (what a client adopts on its first
        // pull) was seeded from the POST-reservation `_pylon_change_seq`
        // value (floor + chunk_size), while the allocator issues seqs
        // from floor + 1. Every delta in the first chunk landed BELOW the
        // cursor, so clients dropped it as "already seen" and live
        // queries silently never updated on `pylon dev` (SQLite). Wire
        // the ChangeLog the way `serve()` does — `with_initial_seq(
        // floor_seq())` — and assert the first appended delta sits ABOVE
        // the cursor a fresh client would adopt.
        use pylon_sync::{ChangeKind, ChangeLog, SeqProvider};

        let tmp = std::env::temp_dir().join("pylon-seq-floor-wiring.sqlite");
        let rt = open_runtime(&tmp);
        let allocator = Arc::new(SqliteSeqAllocator::new_with_chunk(rt.clone(), 100).unwrap());

        let alloc_for_provider = Arc::clone(&allocator);
        let provider: SeqProvider = Arc::new(move || alloc_for_provider.next());
        let log = ChangeLog::new()
            .with_seq(provider)
            .with_initial_seq(allocator.floor_seq());

        // The cursor a freshly-connecting client adopts from the snapshot.
        let snapshot_cursor = log.current_seq();
        let first_seq = log.append(
            "T",
            "row-1",
            ChangeKind::Insert,
            Some(serde_json::json!({ "x": 1 })),
        );

        assert!(
            first_seq > snapshot_cursor,
            "first delta seq ({first_seq}) must be ABOVE the client's snapshot \
             cursor ({snapshot_cursor}); seeding the cursor from the \
             post-reservation _pylon_change_seq value drops every SQLite \
             live-query delta in the first chunk"
        );
        let _ = std::fs::remove_file(&tmp);
    }
}
