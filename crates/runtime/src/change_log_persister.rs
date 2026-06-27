//! Background-thread persister for the SQLite change log.
//!
//! The change log's `append` lives on the mutation hot path. The
//! runtime's SQLite TS-mutation pipeline (datastore.rs around the
//! `BEGIN/COMMIT` scope) already holds the `write_conn` lock while
//! calling `change_log.append`. If `append` then tried to acquire
//! `write_conn` again to persist the event inline, the call would
//! deadlock — `std::sync::Mutex` is not reentrant.
//!
//! Same deadlock shape the seq allocator faces (see
//! [`crate::seq_allocator`]); the answer is the same: move the
//! write off the mutation thread.
//!
//! Shape:
//!   1. The change log's hot path calls `Persister::enqueue(event)`.
//!   2. `enqueue` sends the event through an `mpsc::sync_channel`
//!      sized so the hot path never blocks under normal load.
//!   3. A dedicated worker thread drains the channel, acquires
//!      `write_conn` (queued behind any active mutation tx — same
//!      lock, separate acquisition), and runs a batched
//!      `INSERT` for everything pending.
//!
//! Failure mode: if the worker can't keep up (channel full), the
//! enqueue silently drops the event. The in-memory ring still has
//! it; the disk log lags. We log a warning each time. For
//! chat-volume workloads this never trips; if it does, the operator
//! bumps the channel bound or the runtime configures a wider one.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use pylon_sync::ChangeEvent;

use crate::Runtime;

/// Item flowing through the worker channel. Events are the common case;
/// a `Flush` is a barrier the worker acks only AFTER every event ahead
/// of it in the FIFO has been persisted + committed.
///
/// Putting flushes in the same channel (rather than a side channel the
/// worker would have to `select` on — which std mpsc can't do) gives a
/// clean ordering guarantee for free: everything enqueued before
/// `flush()` was called sits ahead of the sentinel, so by the time the
/// worker reaches it, those events are already on disk.
enum WorkerMsg {
    Event(ChangeEvent),
    /// Persist everything ahead of this barrier, then `send(())` on the
    /// paired channel. Used by clean shutdown (durability) and tests
    /// (determinism over the otherwise-async persist).
    Flush(SyncSender<()>),
}

/// Channel buffer. Each entry is a single `ChangeEvent`. The hot
/// path's `send` is `try_send`; full buffer drops + warns rather
/// than blocking the mutation tx that's holding `write_conn`.
const CHANNEL_BOUND: usize = 16_384;

/// Max events drained per worker iteration. Batching reduces lock
/// acquisitions; too-large batches starve other write_conn users
/// (mutations) for too long. 256 keeps each batch sub-millisecond.
const BATCH_LIMIT: usize = 256;

/// How long the worker sleeps between drain attempts when the
/// channel is idle. Short enough that latency is bounded; long
/// enough that a dormant chat-app doesn't busy-loop a thread.
const IDLE_POLL: Duration = Duration::from_millis(50);

/// Default retention for the persisted change log. Bounds disk
/// growth — the log is append-only and grows linearly with
/// mutation rate. 100k events at typical row sizes is ~10MB.
/// Override via `PYLON_CHANGE_LOG_RETAIN`.
const DEFAULT_RETAIN: u64 = 100_000;

/// How often the prune timer ticks. 5 minutes balances bounded
/// growth between ticks against not over-running write_conn.
const PRUNE_INTERVAL: Duration = Duration::from_secs(300);

/// Observability counters exposed on [`PersisterMetrics`]. Updated
/// inline; queryable via `Persister::metrics()` for tests + the
/// pylon-cloud dashboard. We don't push to Tinybird directly here
/// — the dashboard reads these via the existing trace surface.
#[derive(Debug, Default)]
pub struct PersisterMetrics {
    /// Events enqueued successfully (try_send Ok).
    pub events_enqueued: AtomicU64,
    /// Events DROPPED because the channel was full. P0 prod symptom
    /// — operator alert at any non-zero value.
    pub events_dropped: AtomicU64,
    /// Events persisted to disk (a batched INSERT succeeded).
    pub events_persisted: AtomicU64,
    /// Batches that failed mid-INSERT. Same P0 risk as drops.
    pub batches_failed: AtomicU64,
    /// Successful prune ticks.
    pub prunes_ok: AtomicU64,
    /// Failed prune ticks.
    pub prunes_failed: AtomicU64,
    /// Rows deleted across all prunes.
    pub rows_pruned: AtomicU64,
}

impl PersisterMetrics {
    fn snapshot(&self) -> PersisterMetricsSnapshot {
        PersisterMetricsSnapshot {
            events_enqueued: self.events_enqueued.load(Ordering::Relaxed),
            events_dropped: self.events_dropped.load(Ordering::Relaxed),
            events_persisted: self.events_persisted.load(Ordering::Relaxed),
            batches_failed: self.batches_failed.load(Ordering::Relaxed),
            prunes_ok: self.prunes_ok.load(Ordering::Relaxed),
            prunes_failed: self.prunes_failed.load(Ordering::Relaxed),
            rows_pruned: self.rows_pruned.load(Ordering::Relaxed),
        }
    }
}

/// Plain-data snapshot for serialization / tests.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct PersisterMetricsSnapshot {
    pub events_enqueued: u64,
    pub events_dropped: u64,
    pub events_persisted: u64,
    pub batches_failed: u64,
    pub prunes_ok: u64,
    pub prunes_failed: u64,
    pub rows_pruned: u64,
}

/// Backend-specific persist + prune surface. Both the SQLite path
/// and the Postgres path provide an impl; the worker loop is
/// shared. Method names mirror `Runtime::sqlite_change_log_*` /
/// `pg_change_log_*` directly.
pub trait ChangeLogBackend: Send + Sync + 'static {
    fn persist_batch(&self, events: &[ChangeEvent]) -> Result<(), String>;
    fn prune(&self, retain: u64) -> Result<usize, String>;
    fn name(&self) -> &'static str;
}

/// SQLite backend impl. Backed by `Runtime::sqlite_change_log_*`.
pub struct SqliteBackend {
    pub runtime: Arc<Runtime>,
}

impl ChangeLogBackend for SqliteBackend {
    fn persist_batch(&self, events: &[ChangeEvent]) -> Result<(), String> {
        self.runtime
            .sqlite_change_log_persist_batch(events)
            .map_err(|e| e.message)
    }
    fn prune(&self, retain: u64) -> Result<usize, String> {
        self.runtime
            .sqlite_change_log_prune(retain)
            .map_err(|e| e.message)
    }
    fn name(&self) -> &'static str {
        "sqlite"
    }
}

/// Postgres backend impl. Backed by `Runtime::pg_change_log_*`.
pub struct PgBackend {
    pub runtime: Arc<Runtime>,
}

impl ChangeLogBackend for PgBackend {
    fn persist_batch(&self, events: &[ChangeEvent]) -> Result<(), String> {
        self.runtime
            .pg_change_log_persist_batch(events)
            .map_err(|e| e.message)
    }
    fn prune(&self, retain: u64) -> Result<usize, String> {
        self.runtime
            .pg_change_log_prune(retain)
            .map(|n| n as usize)
            .map_err(|e| e.message)
    }
    fn name(&self) -> &'static str {
        "postgres"
    }
}

/// Send-side handle the change-log hot path holds. Cheap to clone —
/// `tx` and `metrics` are both Arc-shaped. Type-erased over the
/// backend so SQLite + Postgres share one struct.
#[derive(Clone)]
pub struct ChangeLogPersister {
    tx: SyncSender<WorkerMsg>,
    metrics: Arc<PersisterMetrics>,
}

impl std::fmt::Debug for ChangeLogPersister {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeLogPersister").finish()
    }
}

impl ChangeLogPersister {
    /// Spawn the worker thread bound to `backend`. Returns the
    /// send-side handle.
    pub fn spawn(backend: Arc<dyn ChangeLogBackend>) -> Self {
        let (tx, rx) = sync_channel::<WorkerMsg>(CHANNEL_BOUND);
        let metrics = Arc::new(PersisterMetrics::default());
        let metrics_for_worker = Arc::clone(&metrics);
        let retain = std::env::var("PYLON_CHANGE_LOG_RETAIN")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_RETAIN);
        let backend_name = backend.name();
        thread::Builder::new()
            .name(format!("pylon-change-log-persister-{}", backend_name))
            .spawn(move || worker(backend, rx, metrics_for_worker, retain))
            .expect("spawn change-log persister thread");
        Self { tx, metrics }
    }

    /// Convenience: spawn against a SQLite-backed runtime.
    pub fn spawn_sqlite(runtime: Arc<Runtime>) -> Self {
        Self::spawn(Arc::new(SqliteBackend { runtime }))
    }

    /// Convenience: spawn against a Postgres-backed runtime.
    pub fn spawn_pg(runtime: Arc<Runtime>) -> Self {
        Self::spawn(Arc::new(PgBackend { runtime }))
    }

    /// Non-blocking enqueue. Returns immediately. If the channel is
    /// full the event is dropped, the drop counter ticks, and a
    /// warning fires — better than wedging the mutation tx.
    pub fn enqueue(&self, event: ChangeEvent) {
        match self.tx.try_send(WorkerMsg::Event(event)) {
            Ok(()) => {
                self.metrics.events_enqueued.fetch_add(1, Ordering::Relaxed);
            }
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                self.metrics.events_dropped.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    "[change_log] persister channel full; dropping event (raise CHANNEL_BOUND or check disk health)"
                );
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.metrics.events_dropped.fetch_add(1, Ordering::Relaxed);
                tracing::error!("[change_log] persister thread died; events will not be persisted");
            }
        }
    }

    /// Block until every event enqueued before this call has been
    /// persisted + committed to disk, or `timeout` elapses. Returns
    /// `true` if the flush completed, `false` on timeout or a dead
    /// worker.
    ///
    /// Two callers:
    ///   - **Clean shutdown.** The graceful-exit path drains HTTP
    ///     workers, then flushes the change log BEFORE the WAL
    ///     checkpoint, so events still sitting in the channel at exit
    ///     reach disk. Without this, the last few mutations before a
    ///     redeploy are silently lost — the "send a message, restart,
    ///     refresh, it's gone" bug, just narrowed to the shutdown
    ///     window instead of the whole in-memory log.
    ///   - **Tests.** The persister is async by design (off the
    ///     mutation hot path); a lifecycle test that mutates then
    ///     "restarts" needs a barrier to know the write landed before
    ///     it tears down the runtime.
    ///
    /// Mechanism: a `Flush` sentinel rides the same FIFO as events, so
    /// the worker only acks it after everything ahead of it has
    /// committed. No polling, no counter races.
    pub fn flush(&self, timeout: Duration) -> bool {
        let (ack_tx, ack_rx) = sync_channel::<()>(1);
        // Blocking send (not try_send): under a full channel the worker
        // is actively draining, so the barrier gets in shortly. Only a
        // dead worker (all receivers gone) errors here.
        if self.tx.send(WorkerMsg::Flush(ack_tx)).is_err() {
            return false;
        }
        ack_rx.recv_timeout(timeout).is_ok()
    }

    /// Snapshot the current counters for observability. Cheap —
    /// atomic loads with `Relaxed` ordering.
    pub fn metrics(&self) -> PersisterMetricsSnapshot {
        self.metrics.snapshot()
    }
}

/// Worker loop. Drains pending events in batches, runs the inserts
/// inside one tx per batch, periodically prunes. Backend-agnostic.
fn worker(
    backend: Arc<dyn ChangeLogBackend>,
    rx: std::sync::mpsc::Receiver<WorkerMsg>,
    metrics: Arc<PersisterMetrics>,
    retain: u64,
) {
    let mut last_prune_at = Instant::now();
    loop {
        let first = match rx.recv() {
            Ok(m) => m,
            Err(_) => return, // sender dropped — runtime shutting down
        };
        let mut batch: Vec<ChangeEvent> = Vec::with_capacity(BATCH_LIMIT);
        // Flush acks collected this iteration. A `Flush` is a barrier:
        // we stop draining at it so everything ahead of it is in this
        // (or an already-committed prior) batch, then ack AFTER commit.
        let mut acks: Vec<SyncSender<()>> = Vec::new();
        match first {
            WorkerMsg::Event(e) => batch.push(e),
            WorkerMsg::Flush(ack) => acks.push(ack),
        }
        while batch.len() < BATCH_LIMIT {
            match rx.try_recv() {
                Ok(WorkerMsg::Event(e)) => batch.push(e),
                Ok(WorkerMsg::Flush(ack)) => {
                    // Barrier reached — drain no further this round so
                    // the ack strictly follows the commit of everything
                    // before it.
                    acks.push(ack);
                    break;
                }
                Err(_) => break,
            }
        }
        if !batch.is_empty() {
            let batch_size = batch.len() as u64;
            match backend.persist_batch(&batch) {
                Ok(()) => {
                    metrics
                        .events_persisted
                        .fetch_add(batch_size, Ordering::Relaxed);
                }
                Err(e) => {
                    metrics.batches_failed.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        error = %e,
                        count = batch_size,
                        backend = backend.name(),
                        "[change_log] batch persist failed; events kept in-memory only"
                    );
                }
            }
        }

        // Signal any flush barriers now that the batch is committed.
        // Send is non-blocking (ack channel has a free slot) and a
        // dropped receiver (caller already timed out) is ignored.
        for ack in acks {
            let _ = ack.send(());
        }

        if last_prune_at.elapsed() >= PRUNE_INTERVAL {
            last_prune_at = Instant::now();
            match backend.prune(retain) {
                Ok(rows) => {
                    metrics.prunes_ok.fetch_add(1, Ordering::Relaxed);
                    metrics
                        .rows_pruned
                        .fetch_add(rows as u64, Ordering::Relaxed);
                    if rows > 0 {
                        tracing::info!(
                            rows = rows,
                            retain = retain,
                            backend = backend.name(),
                            "[change_log] pruned"
                        );
                    }
                }
                Err(e) => {
                    metrics.prunes_failed.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        error = %e,
                        backend = backend.name(),
                        "[change_log] prune failed"
                    );
                }
            }
        }

        thread::sleep(IDLE_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    fn ev(seq: u64) -> ChangeEvent {
        ChangeEvent {
            seq,
            entity: "Message".into(),
            row_id: format!("r{seq}"),
            kind: pylon_sync::ChangeKind::Insert,
            data: None,
            prev_data: None,
            timestamp: "0Z".into(),
        }
    }

    /// Counts persisted events; optional per-batch delay lets a test
    /// prove `flush` actually waits for an in-flight batch rather than
    /// returning on a stale counter read.
    struct CountingBackend {
        persisted: Arc<AtomicU64>,
        delay: Duration,
    }
    impl ChangeLogBackend for CountingBackend {
        fn persist_batch(&self, events: &[ChangeEvent]) -> Result<(), String> {
            if !self.delay.is_zero() {
                thread::sleep(self.delay);
            }
            self.persisted
                .fetch_add(events.len() as u64, Ordering::SeqCst);
            Ok(())
        }
        fn prune(&self, _retain: u64) -> Result<usize, String> {
            Ok(0)
        }
        fn name(&self) -> &'static str {
            "counting"
        }
    }

    /// The core barrier guarantee: once `flush` returns true, every
    /// event enqueued before it has been persisted — even when the
    /// backend is slow enough that an un-synchronised caller would race
    /// ahead of the worker.
    #[test]
    fn flush_waits_for_all_enqueued_events() {
        let persisted = Arc::new(AtomicU64::new(0));
        let backend = Arc::new(CountingBackend {
            persisted: Arc::clone(&persisted),
            delay: Duration::from_millis(5),
        });
        let p = ChangeLogPersister::spawn(backend);

        for seq in 1..=500 {
            p.enqueue(ev(seq));
        }
        assert!(p.flush(Duration::from_secs(5)), "flush should complete");
        assert_eq!(
            persisted.load(Ordering::SeqCst),
            500,
            "flush must not return until every enqueued event is committed"
        );
    }

    /// A flush with nothing pending returns immediately (still true).
    #[test]
    fn flush_with_empty_queue_succeeds() {
        let persisted = Arc::new(AtomicU64::new(0));
        let backend = Arc::new(CountingBackend {
            persisted,
            delay: Duration::ZERO,
        });
        let p = ChangeLogPersister::spawn(backend);
        assert!(p.flush(Duration::from_secs(1)));
    }

    /// Events enqueued AFTER a flush barrier are not required to be
    /// committed by that flush — only a subsequent flush covers them.
    #[test]
    fn flush_is_a_point_in_time_barrier() {
        let persisted = Arc::new(AtomicU64::new(0));
        let backend = Arc::new(CountingBackend {
            persisted: Arc::clone(&persisted),
            delay: Duration::ZERO,
        });
        let p = ChangeLogPersister::spawn(backend);

        for seq in 1..=10 {
            p.enqueue(ev(seq));
        }
        assert!(p.flush(Duration::from_secs(5)));
        let after_first = persisted.load(Ordering::SeqCst);
        assert_eq!(after_first, 10);

        for seq in 11..=20 {
            p.enqueue(ev(seq));
        }
        assert!(p.flush(Duration::from_secs(5)));
        assert_eq!(persisted.load(Ordering::SeqCst), 20);
    }
}
