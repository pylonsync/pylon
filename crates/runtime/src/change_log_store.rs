//! `ChangeLogStore` impls backed by SQLite + Postgres via [`Runtime`].
//!
//! Both impls share a bg-thread persister (see
//! [`crate::change_log_persister`]) — the hot path holds the
//! write_conn / pg client for its own mutation tx and would deadlock
//! or queue badly if append touched the connection inline.

use std::sync::Arc;

use pylon_sync::{ChangeEvent, ChangeLogStore};

use crate::change_log_persister::ChangeLogPersister;
use crate::Runtime;

/// SQLite-backed store. Reads route directly to the runtime; writes
/// go through the bg-thread persister.
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
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let persister = ChangeLogPersister::spawn_sqlite(Arc::clone(&runtime));
        Self { runtime, persister }
    }

    /// Snapshot the persister's counters for observability.
    pub fn metrics(&self) -> crate::change_log_persister::PersisterMetricsSnapshot {
        self.persister.metrics()
    }
}

impl ChangeLogStore for SqliteChangeLogStore {
    fn append(&self, event: &ChangeEvent) {
        self.persister.enqueue(event.clone());
    }
    fn load_recent(&self, limit: usize) -> Vec<ChangeEvent> {
        self.runtime.sqlite_change_log_load_recent(limit)
    }
    fn pull_range(&self, since: u64, limit: usize) -> Vec<ChangeEvent> {
        self.runtime.sqlite_change_log_pull_range(since, limit)
    }
    fn flush(&self, timeout: std::time::Duration) -> bool {
        self.persister.flush(timeout)
    }
}

/// Postgres-backed store. Same shape as SQLite — bg-thread persister
/// over the runtime's PG client.
pub struct PgChangeLogStore {
    runtime: Arc<Runtime>,
    persister: ChangeLogPersister,
}

impl std::fmt::Debug for PgChangeLogStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgChangeLogStore").finish()
    }
}

impl PgChangeLogStore {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let persister = ChangeLogPersister::spawn_pg(Arc::clone(&runtime));
        Self { runtime, persister }
    }

    pub fn metrics(&self) -> crate::change_log_persister::PersisterMetricsSnapshot {
        self.persister.metrics()
    }
}

impl ChangeLogStore for PgChangeLogStore {
    fn append(&self, event: &ChangeEvent) {
        self.persister.enqueue(event.clone());
    }
    fn load_recent(&self, limit: usize) -> Vec<ChangeEvent> {
        self.runtime.pg_change_log_load_recent(limit)
    }
    fn pull_range(&self, since: u64, limit: usize) -> Vec<ChangeEvent> {
        self.runtime.pg_change_log_pull_range(since, limit)
    }
    fn flush(&self, timeout: std::time::Duration) -> bool {
        self.persister.flush(timeout)
    }
}
