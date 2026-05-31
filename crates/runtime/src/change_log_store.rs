//! `ChangeLogStore` impl backed by SQLite via [`Runtime`].
//!
//! Writes go through [`crate::change_log_persister::ChangeLogPersister`]
//! — a bg-thread persister with a bounded mpsc channel. The change
//! log's hot path enqueues from inside a held `write_conn` lock; the
//! persister thread drains and writes from outside that lock so the
//! mutation never blocks on its own connection (the v0.3.218 +
//! v0.3.224 deadlock shape).
//!
//! Reads (load_recent / pull_range) go directly to the runtime — they
//! happen on cold paths (boot, late-cursor pull) and are not under
//! held locks.

use std::sync::Arc;

use pylon_sync::{ChangeEvent, ChangeLogStore};

use crate::change_log_persister::ChangeLogPersister;
use crate::Runtime;

/// `ChangeLogStore` impl: routes `append` through a bg-thread
/// persister; routes reads through `Runtime`'s direct SQL.
pub struct SqliteChangeLogStore {
    runtime: Arc<Runtime>,
    persister: ChangeLogPersister,
}

impl std::fmt::Debug for SqliteChangeLogStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteChangeLogStore").finish()
    }
}

impl SqliteChangeLogStore {
    /// Spawn the bg-thread persister and return the wired store. The
    /// persister inherits the runtime's lifetime via `Arc::clone`.
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let persister = ChangeLogPersister::spawn(Arc::clone(&runtime));
        Self { runtime, persister }
    }
}

impl ChangeLogStore for SqliteChangeLogStore {
    fn append(&self, event: &ChangeEvent) {
        // Non-blocking handoff. The hot path holds write_conn for its
        // own mutation tx — we MUST NOT touch write_conn from here.
        self.persister.enqueue(event.clone());
    }
    fn load_recent(&self, limit: usize) -> Vec<ChangeEvent> {
        self.runtime.sqlite_change_log_load_recent(limit)
    }
    fn pull_range(&self, since: u64, limit: usize) -> Vec<ChangeEvent> {
        self.runtime.sqlite_change_log_pull_range(since, limit)
    }
}
