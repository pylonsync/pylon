//! Background-thread persister for the SQLite change log.
//!
//! The change log's `append` lives on the mutation hot path. The
//! runtime's SQLite TS-mutation pipeline (datastore.rs around the
//! `BEGIN/COMMIT` scope) already holds the `write_conn` lock while
//! calling `change_log.append`. If `append` then tried to acquire
//! `write_conn` again to persist the event inline, the call would
//! deadlock — `std::sync::Mutex` is not reentrant.
//!
//! Same deadlock shape as the v0.3.218 seq-persistence regression
//! (see [`crate::seq_allocator`]). The fix is the same: move the
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

use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use pylon_sync::ChangeEvent;

use crate::Runtime;

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

/// Send-side handle the change-log hot path holds. Cheap to clone.
#[derive(Clone)]
pub struct ChangeLogPersister {
    tx: SyncSender<ChangeEvent>,
}

impl std::fmt::Debug for ChangeLogPersister {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeLogPersister").finish()
    }
}

impl ChangeLogPersister {
    /// Spawn the worker thread + return the send-side handle. Holds
    /// an `Arc<Runtime>` clone to acquire `write_conn` from the
    /// worker without keeping `runtime` borrowed at the call site.
    pub fn spawn(runtime: Arc<Runtime>) -> Self {
        let (tx, rx) = sync_channel::<ChangeEvent>(CHANNEL_BOUND);
        thread::Builder::new()
            .name("pylon-change-log-persister".into())
            .spawn(move || worker(runtime, rx))
            .expect("spawn change-log persister thread");
        Self { tx }
    }

    /// Non-blocking enqueue. Returns immediately. If the channel is
    /// full the event is dropped and a warning is logged — better
    /// than wedging the mutation tx.
    pub fn enqueue(&self, event: ChangeEvent) {
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                tracing::warn!(
                    "[change_log] persister channel full; dropping event (raise CHANNEL_BOUND or check disk health)"
                );
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                tracing::error!("[change_log] persister thread died; events will not be persisted");
            }
        }
    }
}

/// Worker loop. Drains pending events in batches, acquires
/// `write_conn` ONCE per batch, runs the inserts inside a single
/// SQLite transaction so the round-trip cost is amortized.
fn worker(runtime: Arc<Runtime>, rx: std::sync::mpsc::Receiver<ChangeEvent>) {
    loop {
        // Block until at least one event arrives, then drain up to
        // `BATCH_LIMIT - 1` more without blocking. Bounds the work
        // we hold `write_conn` for per iteration.
        let first = match rx.recv() {
            Ok(e) => e,
            Err(_) => {
                // Sender dropped — runtime is shutting down.
                return;
            }
        };
        let mut batch: Vec<ChangeEvent> = Vec::with_capacity(BATCH_LIMIT);
        batch.push(first);
        while batch.len() < BATCH_LIMIT {
            match rx.try_recv() {
                Ok(e) => batch.push(e),
                Err(_) => break,
            }
        }
        if let Err(e) = persist_batch(&runtime, &batch) {
            tracing::warn!(
                error = %e,
                count = batch.len(),
                "[change_log] batch persist failed; events kept in-memory only"
            );
        }
        // Brief pause before next blocking recv when traffic is
        // spiky — keeps the worker responsive without spinning.
        thread::sleep(IDLE_POLL);
    }
}

/// Run one batched INSERT. Acquires `write_conn` once, runs N
/// INSERTs in a single explicit tx for predictability. Returns an
/// error if the conn isn't available or any INSERT fails.
fn persist_batch(runtime: &Runtime, batch: &[ChangeEvent]) -> Result<(), String> {
    if batch.is_empty() {
        return Ok(());
    }
    runtime
        .sqlite_change_log_persist_batch(batch)
        .map_err(|e| e.message)
}
