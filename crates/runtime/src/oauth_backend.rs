//! Persistent OAuth state stores (SQLite + Postgres).
//!
//! State tokens are short-lived (10 min) and single-use. Persisting
//! them to durable storage lets the OAuth flow survive a server
//! restart that happens between the user clicking "Sign in with
//! Google" and the provider redirecting back. Schema carries the
//! callback / error_callback URLs (validated against PYLON_TRUSTED_ORIGINS
//! at create time) so the callback handler doesn't need any env var
//! to know where to redirect after success or failure.
//!
//! Cleanup happens lazily — when `take()` finds an expired token it
//! returns None and the row sticks around until VACUUM. At the
//! volumes OAuth flows actually generate this is never a problem.

use std::sync::{Arc, Mutex};

use pylon_auth::{OAuthState, OAuthStateBackend};
use rusqlite::Connection;

const TABLE: &str = "_pylon_oauth_state";

// ---------------------------------------------------------------------------
// SQLite backend
// ---------------------------------------------------------------------------

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
        // Base table for new installs. Existing installs predate the
        // callback_url / error_callback_url columns and get an
        // ALTER TABLE ADD COLUMN below — ADD COLUMN is a no-op when
        // the column already exists, so we swallow its error for
        // idempotency. Same pattern as session_backend's tenant_id
        // migration.
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {TABLE} (
                token TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                callback_url TEXT NOT NULL DEFAULT '',
                error_callback_url TEXT NOT NULL DEFAULT '',
                pkce_verifier TEXT,
                expires_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS {TABLE}_exp_idx ON {TABLE}(expires_at);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        let _ = conn.execute(
            &format!("ALTER TABLE {TABLE} ADD COLUMN callback_url TEXT NOT NULL DEFAULT ''"),
            [],
        );
        let _ = conn.execute(
            &format!("ALTER TABLE {TABLE} ADD COLUMN error_callback_url TEXT NOT NULL DEFAULT ''"),
            [],
        );
        // PKCE column was added when Twitter/X support landed. Existing
        // installs need an idempotent ADD COLUMN.
        let _ = conn.execute(
            &format!("ALTER TABLE {TABLE} ADD COLUMN pkce_verifier TEXT"),
            [],
        );
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl OAuthStateBackend for SqliteOAuthBackend {
    fn put(&self, token: &str, state: &OAuthState) {
        if let Ok(guard) = self.conn.lock() {
            let _ = guard.execute(
                &format!(
                    "INSERT INTO {TABLE} (token, provider, callback_url, error_callback_url, pkce_verifier, expires_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(token) DO UPDATE SET
                       provider = excluded.provider,
                       callback_url = excluded.callback_url,
                       error_callback_url = excluded.error_callback_url,
                       pkce_verifier = excluded.pkce_verifier,
                       expires_at = excluded.expires_at"
                ),
                rusqlite::params![
                    token,
                    state.provider,
                    state.callback_url,
                    state.error_callback_url,
                    state.pkce_verifier,
                    state.expires_at as i64,
                ],
            );
        }
    }

    fn take(&self, token: &str, now_unix_secs: u64) -> Option<OAuthState> {
        let guard = self.conn.lock().ok()?;
        // Read first, then delete — must be a transaction so concurrent
        // callbacks can't both succeed with the same token.
        let tx = guard.unchecked_transaction().ok()?;
        let row: Option<(String, String, String, Option<String>, i64)> = tx
            .query_row(
                &format!(
                    "SELECT provider, callback_url, error_callback_url, pkce_verifier, expires_at
                     FROM {TABLE} WHERE token = ?1"
                ),
                rusqlite::params![token],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
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

        let (provider, callback_url, error_callback_url, pkce_verifier, expires_at) = row?;
        if (expires_at as u64) <= now_unix_secs {
            return None;
        }
        Some(OAuthState {
            provider,
            callback_url,
            error_callback_url,
            pkce_verifier,
            expires_at: expires_at as u64,
        })
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
            let mut client = pylon_storage::postgres::live::connect_pg(url)?;
            // Same shape as the SQLite version — declare the columns
            // up front for new installs, and idempotent ALTER TABLEs
            // for ones that predate the callback URL fields. Postgres'
            // IF NOT EXISTS on ADD COLUMN is fine here.
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_TABLE} (
                        token TEXT PRIMARY KEY,
                        provider TEXT NOT NULL,
                        callback_url TEXT NOT NULL DEFAULT '',
                        error_callback_url TEXT NOT NULL DEFAULT '',
                        pkce_verifier TEXT,
                        expires_at BIGINT NOT NULL
                    );
                    ALTER TABLE {PG_TABLE} ADD COLUMN IF NOT EXISTS callback_url TEXT NOT NULL DEFAULT '';
                    ALTER TABLE {PG_TABLE} ADD COLUMN IF NOT EXISTS error_callback_url TEXT NOT NULL DEFAULT '';
                    ALTER TABLE {PG_TABLE} ADD COLUMN IF NOT EXISTS pkce_verifier TEXT;
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_exp_idx ON {PG_TABLE}(expires_at);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl OAuthStateBackend for PostgresOAuthBackend {
        fn put(&self, token: &str, state: &OAuthState) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {PG_TABLE} (token, provider, callback_url, error_callback_url, pkce_verifier, expires_at)
                         VALUES ($1, $2, $3, $4, $5, $6)
                         ON CONFLICT (token) DO UPDATE SET
                           provider = EXCLUDED.provider,
                           callback_url = EXCLUDED.callback_url,
                           error_callback_url = EXCLUDED.error_callback_url,
                           pkce_verifier = EXCLUDED.pkce_verifier,
                           expires_at = EXCLUDED.expires_at"
                    ),
                    &[
                        &token,
                        &state.provider,
                        &state.callback_url,
                        &state.error_callback_url,
                        &state.pkce_verifier,
                        &(state.expires_at as i64),
                    ],
                );
            }
        }

        fn take(&self, token: &str, now_unix_secs: u64) -> Option<OAuthState> {
            // Single round-trip with `RETURNING` is atomic enough — the
            // DELETE removes the row whether it's expired or not (single-use),
            // and we filter the returned state by expires_at after.
            // Concurrent callbacks for the same token can't both succeed
            // because only one DELETE will return a row.
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "DELETE FROM {PG_TABLE} WHERE token = $1
                         RETURNING provider, callback_url, error_callback_url, pkce_verifier, expires_at"
                    ),
                    &[&token],
                )
                .ok()??;
            let provider: String = row.get(0);
            let callback_url: String = row.get(1);
            let error_callback_url: String = row.get(2);
            let pkce_verifier: Option<String> = row.get(3);
            let expires_at: i64 = row.get(4);
            if (expires_at as u64) <= now_unix_secs {
                return None;
            }
            Some(OAuthState {
                provider,
                callback_url,
                error_callback_url,
                pkce_verifier,
                expires_at: expires_at as u64,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(provider: &str, callback: &str) -> OAuthState {
        OAuthState {
            provider: provider.to_string(),
            callback_url: callback.to_string(),
            error_callback_url: callback.to_string(),
            pkce_verifier: None,
            expires_at: 9_999_999_999,
        }
    }

    #[test]
    fn put_then_take_returns_full_state() {
        let b = SqliteOAuthBackend::in_memory().unwrap();
        let s = fixture("google", "http://localhost:3000/dashboard");
        b.put("tok1", &s);
        let got = b.take("tok1", 100).expect("present");
        assert_eq!(got.provider, "google");
        assert_eq!(got.callback_url, "http://localhost:3000/dashboard");
        assert_eq!(got.error_callback_url, "http://localhost:3000/dashboard");
    }

    #[test]
    fn take_is_single_use() {
        let b = SqliteOAuthBackend::in_memory().unwrap();
        b.put("tok2", &fixture("github", "http://localhost:3000/dash"));
        assert!(b.take("tok2", 100).is_some());
        assert!(b.take("tok2", 100).is_none());
    }

    #[test]
    fn expired_token_returns_none() {
        let b = SqliteOAuthBackend::in_memory().unwrap();
        let mut s = fixture("google", "http://localhost:3000/dash");
        s.expires_at = 100;
        b.put("tok3", &s);
        assert!(b.take("tok3", 200).is_none());
    }

    #[test]
    fn missing_token_returns_none() {
        let b = SqliteOAuthBackend::in_memory().unwrap();
        assert!(b.take("never_existed", 0).is_none());
    }
}
