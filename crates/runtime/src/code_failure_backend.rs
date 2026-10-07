//! Durable ledgers of wrong sign-in code guesses, for the per-identifier
//! limit in [`pylon_auth::code_policy`].
//!
//! - [`SqliteCodeFailureLedger`]: same SQLite file as sessions and magic
//!   codes (`PYLON_SESSION_DB`).
//! - [`PostgresCodeFailureLedger`]: shared across replicas when
//!   `DATABASE_URL=postgres://...`.
//!
//! The count must survive a restart or deploy. An in-memory count would
//! reset to zero and give an attacker a new daily budget each time.
//!
//! One row per wrong guess. Rows older than the 24-hour window are pruned
//! on every write, so a table holds at most one window of failures.

use std::sync::{Arc, Mutex};

use pylon_auth::code_policy::CodeFailureLedger;
use rusqlite::Connection;

const TABLE: &str = "_pylon_code_failures";

// ---------------------------------------------------------------------------
// SQLite backend
// ---------------------------------------------------------------------------

pub struct SqliteCodeFailureLedger {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteCodeFailureLedger {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("open: {e}"))?;
        crate::tune_runtime_connection(&conn, false)
            .map_err(|e| format!("pragma init failed: {e}"))?;
        Self::from_connection(conn)
    }

    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open: {e}"))?;
        crate::tune_runtime_connection(&conn, true)
            .map_err(|e| format!("pragma init failed: {e}"))?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {TABLE} (
                key TEXT NOT NULL,
                at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS {TABLE}_key_at_idx ON {TABLE}(key, at);
            CREATE INDEX IF NOT EXISTS {TABLE}_at_idx ON {TABLE}(at);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn exec(&self, what: &str, sql: &str, params: impl rusqlite::Params) {
        match self.conn.lock() {
            Ok(guard) => {
                if let Err(e) = guard.execute(sql, params) {
                    tracing::warn!("[sqlite] code-failure {what} failed: {e}");
                }
            }
            Err(_) => tracing::warn!("[sqlite] code-failure {what}: lock poisoned"),
        }
    }
}

impl CodeFailureLedger for SqliteCodeFailureLedger {
    fn record(&self, key: &str, at: u64) {
        self.exec(
            "record",
            &format!("INSERT INTO {TABLE} (key, at) VALUES (?1, ?2)"),
            rusqlite::params![key, at as i64],
        );
    }
    fn count_since(&self, key: &str, since: u64) -> u32 {
        let Ok(guard) = self.conn.lock() else {
            return 0;
        };
        guard
            .query_row(
                &format!("SELECT COUNT(*) FROM {TABLE} WHERE key = ?1 AND at >= ?2"),
                rusqlite::params![key, since as i64],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as u32)
            .unwrap_or_else(|e| {
                tracing::warn!("[sqlite] code-failure count failed: {e}");
                0
            })
    }
    fn oldest_since(&self, key: &str, since: u64) -> Option<u64> {
        let guard = self.conn.lock().ok()?;
        guard
            .query_row(
                &format!("SELECT MIN(at) FROM {TABLE} WHERE key = ?1 AND at >= ?2"),
                rusqlite::params![key, since as i64],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
            .map(|t| t as u64)
    }
    fn clear(&self, key: &str) {
        self.exec(
            "clear",
            &format!("DELETE FROM {TABLE} WHERE key = ?1"),
            rusqlite::params![key],
        );
    }
    fn prune(&self, before: u64) {
        self.exec(
            "prune",
            &format!("DELETE FROM {TABLE} WHERE at < ?1"),
            rusqlite::params![before as i64],
        );
    }
}

// ---------------------------------------------------------------------------
// Postgres backend
// ---------------------------------------------------------------------------

pub use pg::PostgresCodeFailureLedger;

mod pg {
    use super::*;
    use pylon_storage::pg_datastore::PgPool;

    pub struct PostgresCodeFailureLedger {
        conn: Arc<PgPool>,
    }

    impl PostgresCodeFailureLedger {
        pub fn with_pool(conn: Arc<PgPool>) -> Result<Self, String> {
            conn.with_client(|c| {
                c.batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {TABLE} (
                        key TEXT NOT NULL,
                        at BIGINT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS {TABLE}_key_at_idx ON {TABLE}(key, at);
                    CREATE INDEX IF NOT EXISTS {TABLE}_at_idx ON {TABLE}(at);"
                ))
            })
            .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self { conn })
        }
    }

    impl CodeFailureLedger for PostgresCodeFailureLedger {
        fn record(&self, key: &str, at: u64) {
            if let Err(e) = self.conn.with_client(|c| {
                c.execute(
                    &format!("INSERT INTO {TABLE} (key, at) VALUES ($1, $2)"),
                    &[&key, &(at as i64)],
                )
            }) {
                tracing::warn!("[pg] code-failure record failed: {e}");
            }
        }
        fn count_since(&self, key: &str, since: u64) -> u32 {
            match self.conn.with_client(|c| {
                c.query_one(
                    &format!("SELECT COUNT(*) FROM {TABLE} WHERE key = $1 AND at >= $2"),
                    &[&key, &(since as i64)],
                )
            }) {
                Ok(row) => row.get::<_, i64>(0) as u32,
                Err(e) => {
                    tracing::warn!("[pg] code-failure count failed: {e}");
                    0
                }
            }
        }
        fn oldest_since(&self, key: &str, since: u64) -> Option<u64> {
            match self.conn.with_client(|c| {
                c.query_one(
                    &format!("SELECT MIN(at) FROM {TABLE} WHERE key = $1 AND at >= $2"),
                    &[&key, &(since as i64)],
                )
            }) {
                Ok(row) => row.get::<_, Option<i64>>(0).map(|t| t as u64),
                Err(e) => {
                    tracing::warn!("[pg] code-failure oldest failed: {e}");
                    None
                }
            }
        }
        fn clear(&self, key: &str) {
            if let Err(e) = self
                .conn
                .with_client(|c| c.execute(&format!("DELETE FROM {TABLE} WHERE key = $1"), &[&key]))
            {
                tracing::warn!("[pg] code-failure clear failed: {e}");
            }
        }
        fn prune(&self, before: u64) {
            if let Err(e) = self.conn.with_client(|c| {
                c.execute(
                    &format!("DELETE FROM {TABLE} WHERE at < $1"),
                    &[&(before as i64)],
                )
            }) {
                tracing::warn!("[pg] code-failure prune failed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_auth::code_policy::{CodeGuard, CodePolicy, FAILURE_WINDOW_SECS};

    #[test]
    fn sqlite_records_counts_clears_and_prunes() {
        let l = SqliteCodeFailureLedger::in_memory().unwrap();
        l.record("email:a@b.com", 100);
        l.record("email:a@b.com", 200);
        l.record("phone:+15551234567", 150);
        assert_eq!(l.count_since("email:a@b.com", 0), 2);
        assert_eq!(l.count_since("email:a@b.com", 150), 1);
        assert_eq!(l.oldest_since("email:a@b.com", 0), Some(100));
        assert_eq!(l.oldest_since("email:none@b.com", 0), None);
        l.prune(160);
        assert_eq!(l.count_since("email:a@b.com", 0), 1);
        assert_eq!(l.count_since("phone:+15551234567", 0), 0);
        l.clear("email:a@b.com");
        assert_eq!(l.count_since("email:a@b.com", 0), 0);
    }

    #[test]
    fn sqlite_failures_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let path = path.to_str().unwrap();
        {
            let l = SqliteCodeFailureLedger::open(path).unwrap();
            for i in 0..10 {
                l.record("email:a@b.com", 1_000 + i);
            }
        }
        let l = SqliteCodeFailureLedger::open(path).unwrap();
        assert_eq!(l.count_since("email:a@b.com", 0), 10);
        let guard = CodeGuard::new(CodePolicy::new(4).unwrap(), Arc::new(l));
        assert!(guard.locked_for("email:a@b.com", 2_000).is_some());
        assert_eq!(
            guard.locked_for("email:a@b.com", 1_001 + FAILURE_WINDOW_SECS),
            None
        );
    }

    #[test]
    fn sqlite_ledger_locks_a_magic_code_store() {
        let ledger: Arc<dyn CodeFailureLedger> =
            Arc::new(SqliteCodeFailureLedger::in_memory().unwrap());
        let store = pylon_auth::MagicCodeStore::new()
            .with_guard(CodeGuard::new(CodePolicy::new(4).unwrap(), ledger.clone()));
        let code = store.try_create("a@b.com").unwrap();
        let bad = if code == "0000" { "0001" } else { "0000" };
        for _ in 0..5 {
            let _ = store.try_verify("a@b.com", bad);
        }
        assert_eq!(ledger.count_since("email:a@b.com", 0), 5);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        for _ in 0..5 {
            ledger.record("email:a@b.com", now);
        }
        assert!(matches!(
            store.try_create("a@b.com"),
            Err(pylon_auth::MagicCodeError::Locked { .. })
        ));
    }
}
