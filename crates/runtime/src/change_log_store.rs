//! `ChangeLogStore` impl backed by SQLite via [`Runtime`].
//!
//! Implementation lives on `Runtime` (`bootstrap_sqlite_change_log`,
//! `sqlite_change_log_append`, `sqlite_change_log_load_recent`,
//! `sqlite_change_log_pull_range`) so the SQL stays next to the rest
//! of the SQLite plumbing. This module is a thin wrapper that holds
//! the `Arc<Runtime>` and forwards the trait calls — that lets
//! `pylon-sync` stay free of an `rusqlite` dep.

use std::sync::Arc;

use pylon_sync::{ChangeEvent, ChangeLogStore};

use crate::Runtime;

/// Bind a `ChangeLogStore` to the SQLite tables on this runtime.
/// Hold one `Arc` lifetime past the `ChangeLog` it's attached to —
/// the store is consulted on every mutation's `append` and on every
/// `/api/sync/pull` cursor that falls behind the in-memory ring.
pub struct SqliteChangeLogStore {
    runtime: Arc<Runtime>,
}

impl std::fmt::Debug for SqliteChangeLogStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteChangeLogStore").finish()
    }
}

impl SqliteChangeLogStore {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self { runtime }
    }
}

impl ChangeLogStore for SqliteChangeLogStore {
    fn append(&self, event: &ChangeEvent) {
        self.runtime.sqlite_change_log_append(event);
    }
    fn load_recent(&self, limit: usize) -> Vec<ChangeEvent> {
        self.runtime.sqlite_change_log_load_recent(limit)
    }
    fn pull_range(&self, since: u64, limit: usize) -> Vec<ChangeEvent> {
        self.runtime.sqlite_change_log_pull_range(since, limit)
    }
}
