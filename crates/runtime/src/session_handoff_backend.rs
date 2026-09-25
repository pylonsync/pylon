//! Persistent stores for OAuth session handoffs (SQLite + Postgres).
//!
//! A handoff is minted on the machine that served the OAuth callback and
//! redeemed on whichever machine the tenant host's next request reaches, so
//! it must live in shared storage, like the OAuth state it follows. Rows are
//! keyed by the SHA-256 of the code (see pylon_auth::session_handoff), last
//! 120 seconds, and are deleted by the read that redeems them. An expired row
//! that nobody redeems stays until a later redemption sweeps it.

use std::sync::{Arc, Mutex};

use pylon_auth::session_handoff::{SessionHandoff, SessionHandoffBackend};
use rusqlite::Connection;

const TABLE: &str = "_pylon_session_handoff";

pub struct SqliteSessionHandoffBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteSessionHandoffBackend {
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
                code_hash TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                target_host TEXT NOT NULL,
                redirect_url TEXT NOT NULL,
                binding TEXT,
                expires_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS {TABLE}_exp_idx ON {TABLE}(expires_at);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl SessionHandoffBackend for SqliteSessionHandoffBackend {
    fn put(&self, key: &str, h: &SessionHandoff) {
        let guard = match self.conn.lock() {
            Ok(g) => g,
            Err(e) => {
                tracing::error!(error = %e, "[session-handoff] SQLite mutex poisoned on put — the handoff will fail");
                return;
            }
        };
        if let Err(e) = guard.execute(
            &format!(
                "INSERT INTO {TABLE} (code_hash, user_id, target_host, redirect_url, binding, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
            ),
            rusqlite::params![key, h.user_id, h.target_host, h.redirect_url, h.binding, h.expires_at as i64],
        ) {
            tracing::error!(error = %e, "[session-handoff] SQLite put failed — the handoff will fail");
        }
    }

    fn take(&self, key: &str, now_unix_secs: u64) -> Option<SessionHandoff> {
        let guard = self.conn.lock().ok()?;
        let tx = guard.unchecked_transaction().ok()?;
        #[allow(clippy::type_complexity)]
        let row: Option<(String, String, String, Option<String>, i64)> = tx
            .query_row(
                &format!(
                    "SELECT user_id, target_host, redirect_url, binding, expires_at FROM {TABLE} WHERE code_hash = ?1"
                ),
                rusqlite::params![key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .ok();
        // Single use: delete what was read, and sweep anything expired.
        let _ = tx.execute(
            &format!("DELETE FROM {TABLE} WHERE code_hash = ?1 OR expires_at <= ?2"),
            rusqlite::params![key, now_unix_secs as i64],
        );
        let _ = tx.commit();
        let (user_id, target_host, redirect_url, binding, expires_at) = row?;
        if (expires_at as u64) <= now_unix_secs {
            return None;
        }
        Some(SessionHandoff {
            user_id,
            target_host,
            redirect_url,
            binding,
            expires_at: expires_at as u64,
        })
    }
}

pub use pg::PostgresSessionHandoffBackend;

mod pg {
    use super::*;
    use pylon_storage::pg_datastore::PgPool;

    pub struct PostgresSessionHandoffBackend {
        conn: std::sync::Arc<PgPool>,
    }

    impl PostgresSessionHandoffBackend {
        pub fn with_pool(conn: std::sync::Arc<PgPool>) -> Result<Self, String> {
            conn.with_client(|c| {
                c.batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {TABLE} (
                        code_hash TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        target_host TEXT NOT NULL,
                        redirect_url TEXT NOT NULL,
                        binding TEXT,
                        expires_at BIGINT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS {TABLE}_exp_idx ON {TABLE}(expires_at);"
                ))
            })
            .map_err(|e| format!("PG init schema: {e}"))?;
            // Same boot probe as the OAuth state backend: a write that
            // silently fails would turn every tenant sign-in into a dead end.
            let probe = "_pylon_probe_session_handoff";
            conn.with_client(|c| {
                c.execute(
                    &format!(
                        "INSERT INTO {TABLE} (code_hash, user_id, target_host, redirect_url, expires_at)
                         VALUES ($1, '', '', '', 0) ON CONFLICT (code_hash) DO UPDATE SET expires_at = 0"
                    ),
                    &[&probe],
                )
            })
            .map_err(|e| format!("PG session handoff probe insert: {e}"))?;
            conn.with_client(|c| {
                c.execute(
                    &format!("DELETE FROM {TABLE} WHERE code_hash = $1"),
                    &[&probe],
                )
            })
            .map_err(|e| format!("PG session handoff probe cleanup: {e}"))?;
            Ok(Self { conn })
        }
    }

    impl SessionHandoffBackend for PostgresSessionHandoffBackend {
        fn put(&self, key: &str, h: &SessionHandoff) {
            if let Err(e) = self.conn.with_client(|c| {
                c.execute(
                    &format!(
                        "INSERT INTO {TABLE} (code_hash, user_id, target_host, redirect_url, binding, expires_at)
                         VALUES ($1, $2, $3, $4, $5, $6)"
                    ),
                    &[&key, &h.user_id, &h.target_host, &h.redirect_url, &h.binding, &(h.expires_at as i64)],
                )
            }) {
                tracing::error!(error = %e, "[session-handoff] PG put failed — the handoff will fail");
            }
        }

        fn take(&self, key: &str, now_unix_secs: u64) -> Option<SessionHandoff> {
            // DELETE … RETURNING is the single-use gate: of two concurrent
            // redemptions only one gets the row.
            let row = self
                .conn
                .with_client(|c| {
                    c.query_opt(
                        &format!(
                            "DELETE FROM {TABLE} WHERE code_hash = $1
                             RETURNING user_id, target_host, redirect_url, binding, expires_at"
                        ),
                        &[&key],
                    )
                })
                .ok()
                .flatten();
            let _ = self.conn.with_client(|c| {
                c.execute(
                    &format!("DELETE FROM {TABLE} WHERE expires_at <= $1"),
                    &[&(now_unix_secs as i64)],
                )
            });
            let row = row?;
            let expires_at: i64 = row.get(4);
            if (expires_at as u64) <= now_unix_secs {
                return None;
            }
            Some(SessionHandoff {
                user_id: row.get(0),
                target_host: row.get(1),
                redirect_url: row.get(2),
                binding: row.get(3),
                expires_at: expires_at as u64,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(expires_at: u64) -> SessionHandoff {
        SessionHandoff {
            user_id: "u_1".into(),
            target_host: "feedback.acme.com".into(),
            redirect_url: "https://feedback.acme.com/p".into(),
            binding: Some("h".into()),
            expires_at,
        }
    }

    #[test]
    fn put_then_take_once() {
        let b = SqliteSessionHandoffBackend::in_memory().unwrap();
        b.put("k1", &record(2_000));
        assert_eq!(b.take("k1", 1_000), Some(record(2_000)));
        assert_eq!(b.take("k1", 1_000), None);
    }

    #[test]
    fn expired_is_none_and_swept() {
        let b = SqliteSessionHandoffBackend::in_memory().unwrap();
        b.put("old", &record(500));
        b.put("stale", &record(600));
        assert_eq!(b.take("old", 1_000), None);
        // The sweep in the first take removed the other expired row too.
        let n: i64 = b
            .conn
            .lock()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {TABLE}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn works_through_the_store() {
        let store = pylon_auth::session_handoff::SessionHandoffStore::with_backend(Box::new(
            SqliteSessionHandoffBackend::in_memory().unwrap(),
        ));
        let (cookie, hash) = pylon_auth::session_handoff::new_binding();
        let code = store.create(
            "u_9",
            "feedback.acme.com",
            "https://feedback.acme.com/",
            Some(&hash),
        );
        let got = store
            .redeem(&code, "feedback.acme.com", Some(&cookie))
            .unwrap();
        assert_eq!(got.user_id, "u_9");
        assert_eq!(got.binding.as_deref(), Some(hash.as_str()));
    }
}
