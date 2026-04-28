//! SQLite-backed OAuth state persistence.
//!
//! State tokens are short-lived (10 min) and single-use. Persisting them to
//! SQLite lets the OAuth flow survive a server restart that happens between
//! the user clicking "Sign in with Google" and the provider redirecting back.
//!
//! Schema is one row per token. Cleanup happens lazily — when `take()` finds
//! an expired token it returns None; a periodic VACUUM is unnecessary at the
//! volumes OAuth flows actually generate.

use std::sync::{Arc, Mutex};

use pylon_auth::OAuthStateBackend;
use rusqlite::Connection;

const TABLE: &str = "_pylon_oauth_state";

pub struct SqliteOAuthBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteOAuthBackend {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("open: {e}"))?;
        Self::from_connection(conn)
    }

    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open: {e}"))?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {TABLE} (
                token TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
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

impl OAuthStateBackend for SqliteOAuthBackend {
    fn put(&self, token: &str, provider: &str, expires_at: u64) {
        if let Ok(guard) = self.conn.lock() {
            let _ = guard.execute(
                &format!(
                    "INSERT INTO {TABLE} (token, provider, expires_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(token) DO UPDATE SET
                       provider = excluded.provider,
                       expires_at = excluded.expires_at"
                ),
                rusqlite::params![token, provider, expires_at as i64],
            );
        }
    }

    fn take(&self, token: &str, now_unix_secs: u64) -> Option<String> {
        let guard = self.conn.lock().ok()?;
        // Read first, then delete — must be a transaction so concurrent
        // callbacks can't both succeed with the same token.
        let tx = guard.unchecked_transaction().ok()?;
        let row: Option<(String, i64)> = tx
            .query_row(
                &format!("SELECT provider, expires_at FROM {TABLE} WHERE token = ?1"),
                rusqlite::params![token],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        // Always delete what we read — single-use even if expired.
        if row.is_some() {
            let _ = tx.execute(
                &format!("DELETE FROM {TABLE} WHERE token = ?1"),
                rusqlite::params![token],
            );
        }
        let _ = tx.commit();

        let (provider, expires_at) = row?;
        if (expires_at as u64) <= now_unix_secs {
            return None;
        }
        Some(provider)
    }
}

// ---------------------------------------------------------------------------
// Postgres backend
// ---------------------------------------------------------------------------

pub use pg::PostgresOAuthBackend;

mod pg {
    use super::*;
    use postgres::Client;
    use std::sync::Mutex;

    const PG_TABLE: &str = "_pylon_oauth_state";

    pub struct PostgresOAuthBackend {
        client: Mutex<Client>,
    }

    impl PostgresOAuthBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client =
                Client::connect(url, postgres::NoTls).map_err(|e| format!("PG connect: {e}"))?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_TABLE} (
                        token TEXT PRIMARY KEY,
                        provider TEXT NOT NULL,
                        expires_at BIGINT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_exp_idx ON {PG_TABLE}(expires_at);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl OAuthStateBackend for PostgresOAuthBackend {
        fn put(&self, token: &str, provider: &str, expires_at: u64) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {PG_TABLE} (token, provider, expires_at) VALUES ($1, $2, $3)
                         ON CONFLICT (token) DO UPDATE SET
                           provider = EXCLUDED.provider,
                           expires_at = EXCLUDED.expires_at"
                    ),
                    &[&token, &provider, &(expires_at as i64)],
                );
            }
        }

        fn take(&self, token: &str, now_unix_secs: u64) -> Option<String> {
            // Single round-trip with `RETURNING` is atomic enough — the
            // DELETE removes the row whether it's expired or not (single-use),
            // and we filter the returned provider by expires_at after.
            // Concurrent callbacks for the same token can't both succeed
            // because only one DELETE will return a row.
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "DELETE FROM {PG_TABLE} WHERE token = $1
                         RETURNING provider, expires_at"
                    ),
                    &[&token],
                )
                .ok()??;
            let provider: String = row.get(0);
            let expires_at: i64 = row.get(1);
            if (expires_at as u64) <= now_unix_secs {
                return None;
            }
            Some(provider)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_take_returns_provider() {
        let b = SqliteOAuthBackend::in_memory().unwrap();
        b.put("tok1", "google", 9999999999);
        assert_eq!(b.take("tok1", 100).as_deref(), Some("google"));
    }

    #[test]
    fn take_is_single_use() {
        let b = SqliteOAuthBackend::in_memory().unwrap();
        b.put("tok2", "github", 9999999999);
        assert!(b.take("tok2", 100).is_some());
        assert!(b.take("tok2", 100).is_none());
    }

    #[test]
    fn expired_token_returns_none() {
        let b = SqliteOAuthBackend::in_memory().unwrap();
        b.put("tok3", "google", 100);
        assert!(b.take("tok3", 200).is_none());
    }

    #[test]
    fn missing_token_returns_none() {
        let b = SqliteOAuthBackend::in_memory().unwrap();
        assert!(b.take("never_existed", 0).is_none());
    }

    #[test]
    fn put_overwrites_previous_token() {
        let b = SqliteOAuthBackend::in_memory().unwrap();
        b.put("dup", "google", 9999999999);
        b.put("dup", "github", 9999999999);
        assert_eq!(b.take("dup", 100).as_deref(), Some("github"));
    }
}
