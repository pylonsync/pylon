//! Persistent account-link stores. Schema mirrors better-auth's
//! [`account` table](https://www.better-auth.com/docs/concepts/database)
//! so apps migrating between the two see the same field names + meanings.
//!
//! - [`SqliteAccountBackend`] — single-file durability for self-hosted
//!   single-replica deploys. Lives alongside sessions/oauth-state in
//!   `PYLON_SESSION_DB`.
//! - [`PostgresAccountBackend`] — for `DATABASE_URL=postgres://...`
//!   deploys. Stores account rows in the same Postgres cluster as the
//!   user's data so a join across `_pylon_accounts` and the manifest's
//!   `User` entity works without cross-database queries.
//!
//! Both implement [`pylon_auth::AccountBackend`]. Why a dedicated table
//! instead of folding the link into the manifest's `User` entity:
//!
//! 1. **Multi-provider**: a single user can link Google + GitHub +
//!    custom IdPs + a password (`provider_id="credential"`). The `User`
//!    row needs one identity-of-record (email), not N nullable
//!    provider columns.
//! 2. **Refresh token + password storage**: provider secrets and
//!    password hashes shouldn't be in the user-visible entity surface
//!    — hiding them in a framework table keeps them out of
//!    `/api/entities/User` responses by default.
//! 3. **Schema agility**: the framework owns the account schema, so
//!    adding a new provider column doesn't require a manifest
//!    migration in every consumer app.

use std::sync::{Arc, Mutex};

use pylon_auth::{Account, AccountBackend};
use rusqlite::Connection;

const SQLITE_TABLE: &str = "_pylon_accounts";
const PG_TABLE: &str = "_pylon_accounts";

// ---------------------------------------------------------------------------
// SQLite backend
// ---------------------------------------------------------------------------

pub struct SqliteAccountBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteAccountBackend {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("open: {e}"))?;
        Self::from_connection(conn)
    }

    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open: {e}"))?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, String> {
        // `id` is the row PK; `(provider_id, account_id)` is a UNIQUE
        // composite for the OAuth lookup path. Lookups by user_id hit
        // the secondary index — typically called on /api/auth/me to
        // render "connected providers" UI.
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {SQLITE_TABLE} (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                access_token TEXT,
                refresh_token TEXT,
                id_token TEXT,
                access_token_expires_at INTEGER,
                refresh_token_expires_at INTEGER,
                scope TEXT,
                password TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE (provider_id, account_id)
            );
            CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_user_idx ON {SQLITE_TABLE}(user_id);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn row_to_account(
    id: String,
    user_id: String,
    provider_id: String,
    account_id: String,
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    access_token_expires_at: Option<i64>,
    refresh_token_expires_at: Option<i64>,
    scope: Option<String>,
    password: Option<String>,
    created_at: i64,
    updated_at: i64,
) -> Account {
    Account {
        id,
        user_id,
        provider_id,
        account_id,
        access_token,
        refresh_token,
        id_token,
        access_token_expires_at: access_token_expires_at.map(|n| n as u64),
        refresh_token_expires_at: refresh_token_expires_at.map(|n| n as u64),
        scope,
        password,
        created_at: created_at as u64,
        updated_at: updated_at as u64,
    }
}

const SELECT_COLS: &str = "id, user_id, provider_id, account_id, access_token, \
    refresh_token, id_token, access_token_expires_at, refresh_token_expires_at, \
    scope, password, created_at, updated_at";

impl AccountBackend for SqliteAccountBackend {
    fn upsert(&self, a: &Account) {
        if let Ok(guard) = self.conn.lock() {
            // ON CONFLICT on the composite key — preserves the original
            // row id so external references stay valid across
            // re-authentications.
            let _ = guard.execute(
                &format!(
                    "INSERT INTO {SQLITE_TABLE}
                       (id, user_id, provider_id, account_id, access_token, refresh_token,
                        id_token, access_token_expires_at, refresh_token_expires_at,
                        scope, password, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                     ON CONFLICT(provider_id, account_id) DO UPDATE SET
                       user_id = excluded.user_id,
                       access_token = excluded.access_token,
                       refresh_token = excluded.refresh_token,
                       id_token = excluded.id_token,
                       access_token_expires_at = excluded.access_token_expires_at,
                       refresh_token_expires_at = excluded.refresh_token_expires_at,
                       scope = excluded.scope,
                       password = excluded.password,
                       updated_at = excluded.updated_at"
                ),
                rusqlite::params![
                    a.id,
                    a.user_id,
                    a.provider_id,
                    a.account_id,
                    a.access_token,
                    a.refresh_token,
                    a.id_token,
                    a.access_token_expires_at.map(|n| n as i64),
                    a.refresh_token_expires_at.map(|n| n as i64),
                    a.scope,
                    a.password,
                    a.created_at as i64,
                    a.updated_at as i64,
                ],
            );
        }
    }

    fn find_by_provider(&self, provider_id: &str, account_id: &str) -> Option<Account> {
        let guard = self.conn.lock().ok()?;
        guard
            .query_row(
                &format!(
                    "SELECT {SELECT_COLS}
                     FROM {SQLITE_TABLE}
                     WHERE provider_id = ?1 AND account_id = ?2"
                ),
                rusqlite::params![provider_id, account_id],
                |row| {
                    Ok(row_to_account(
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get(11)?,
                        row.get(12)?,
                    ))
                },
            )
            .ok()
    }

    fn find_for_user(&self, user_id: &str) -> Vec<Account> {
        let Ok(guard) = self.conn.lock() else {
            return Vec::new();
        };
        let mut stmt = match guard.prepare(&format!(
            "SELECT {SELECT_COLS} FROM {SQLITE_TABLE} WHERE user_id = ?1"
        )) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let iter = match stmt.query_map(rusqlite::params![user_id], |row| {
            Ok(row_to_account(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get(11)?,
                row.get(12)?,
            ))
        }) {
            Ok(i) => i,
            Err(_) => return Vec::new(),
        };
        iter.flatten().collect()
    }

    fn unlink(&self, provider_id: &str, account_id: &str) -> bool {
        let Ok(guard) = self.conn.lock() else {
            return false;
        };
        guard
            .execute(
                &format!("DELETE FROM {SQLITE_TABLE} WHERE provider_id = ?1 AND account_id = ?2"),
                rusqlite::params![provider_id, account_id],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    fn list_all(&self) -> Vec<Account> {
        let Ok(guard) = self.conn.lock() else {
            return Vec::new();
        };
        let mut stmt = match guard.prepare(&format!("SELECT {SELECT_COLS} FROM {SQLITE_TABLE}")) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let iter = match stmt.query_map([], |row| {
            Ok(row_to_account(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get(11)?,
                row.get(12)?,
            ))
        }) {
            Ok(i) => i,
            Err(_) => return Vec::new(),
        };
        iter.flatten().collect()
    }
}

// ---------------------------------------------------------------------------
// Postgres backend
// ---------------------------------------------------------------------------

pub use pg::PostgresAccountBackend;

mod pg {
    use super::*;
    use postgres::Client;

    pub struct PostgresAccountBackend {
        client: Mutex<Client>,
    }

    impl PostgresAccountBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client = pylon_storage::postgres::live::connect_pg(url)?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_TABLE} (
                        id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        provider_id TEXT NOT NULL,
                        account_id TEXT NOT NULL,
                        access_token TEXT,
                        refresh_token TEXT,
                        id_token TEXT,
                        access_token_expires_at BIGINT,
                        refresh_token_expires_at BIGINT,
                        scope TEXT,
                        password TEXT,
                        created_at BIGINT NOT NULL,
                        updated_at BIGINT NOT NULL,
                        UNIQUE (provider_id, account_id)
                    );
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_user_idx ON {PG_TABLE}(user_id);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl AccountBackend for PostgresAccountBackend {
        fn upsert(&self, a: &Account) {
            let Ok(mut c) = self.client.lock() else {
                return;
            };
            let _ = c.execute(
                &format!(
                    "INSERT INTO {PG_TABLE}
                       (id, user_id, provider_id, account_id, access_token, refresh_token,
                        id_token, access_token_expires_at, refresh_token_expires_at,
                        scope, password, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                     ON CONFLICT (provider_id, account_id) DO UPDATE SET
                       user_id = EXCLUDED.user_id,
                       access_token = EXCLUDED.access_token,
                       refresh_token = EXCLUDED.refresh_token,
                       id_token = EXCLUDED.id_token,
                       access_token_expires_at = EXCLUDED.access_token_expires_at,
                       refresh_token_expires_at = EXCLUDED.refresh_token_expires_at,
                       scope = EXCLUDED.scope,
                       password = EXCLUDED.password,
                       updated_at = EXCLUDED.updated_at"
                ),
                &[
                    &a.id,
                    &a.user_id,
                    &a.provider_id,
                    &a.account_id,
                    &a.access_token,
                    &a.refresh_token,
                    &a.id_token,
                    &a.access_token_expires_at.map(|n| n as i64),
                    &a.refresh_token_expires_at.map(|n| n as i64),
                    &a.scope,
                    &a.password,
                    &(a.created_at as i64),
                    &(a.updated_at as i64),
                ],
            );
        }

        fn find_by_provider(&self, provider_id: &str, account_id: &str) -> Option<Account> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT {SELECT_COLS}
                         FROM {PG_TABLE}
                         WHERE provider_id = $1 AND account_id = $2"
                    ),
                    &[&provider_id, &account_id],
                )
                .ok()??;
            Some(row_to_account(
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get::<_, Option<String>>(4),
                row.get::<_, Option<String>>(5),
                row.get::<_, Option<String>>(6),
                row.get::<_, Option<i64>>(7),
                row.get::<_, Option<i64>>(8),
                row.get::<_, Option<String>>(9),
                row.get::<_, Option<String>>(10),
                row.get(11),
                row.get(12),
            ))
        }

        fn find_for_user(&self, user_id: &str) -> Vec<Account> {
            let Ok(mut c) = self.client.lock() else {
                return Vec::new();
            };
            let rows = c
                .query(
                    &format!("SELECT {SELECT_COLS} FROM {PG_TABLE} WHERE user_id = $1"),
                    &[&user_id],
                )
                .unwrap_or_default();
            rows.iter()
                .map(|row| {
                    row_to_account(
                        row.get(0),
                        row.get(1),
                        row.get(2),
                        row.get(3),
                        row.get::<_, Option<String>>(4),
                        row.get::<_, Option<String>>(5),
                        row.get::<_, Option<String>>(6),
                        row.get::<_, Option<i64>>(7),
                        row.get::<_, Option<i64>>(8),
                        row.get::<_, Option<String>>(9),
                        row.get::<_, Option<String>>(10),
                        row.get(11),
                        row.get(12),
                    )
                })
                .collect()
        }

        fn unlink(&self, provider_id: &str, account_id: &str) -> bool {
            let Ok(mut c) = self.client.lock() else {
                return false;
            };
            c.execute(
                &format!("DELETE FROM {PG_TABLE} WHERE provider_id = $1 AND account_id = $2"),
                &[&provider_id, &account_id],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
        }

        fn list_all(&self) -> Vec<Account> {
            let Ok(mut c) = self.client.lock() else {
                return Vec::new();
            };
            let rows = c
                .query(&format!("SELECT {SELECT_COLS} FROM {PG_TABLE}"), &[])
                .unwrap_or_default();
            rows.iter()
                .map(|row| {
                    row_to_account(
                        row.get(0),
                        row.get(1),
                        row.get(2),
                        row.get(3),
                        row.get::<_, Option<String>>(4),
                        row.get::<_, Option<String>>(5),
                        row.get::<_, Option<String>>(6),
                        row.get::<_, Option<i64>>(7),
                        row.get::<_, Option<i64>>(8),
                        row.get::<_, Option<String>>(9),
                        row.get::<_, Option<String>>(10),
                        row.get(11),
                        row.get(12),
                    )
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_auth::{Account, AccountBackend};

    fn fixture(provider_id: &str, user: &str, account_id: &str) -> Account {
        Account {
            id: format!("acct_{provider_id}_{account_id}"),
            user_id: user.into(),
            provider_id: provider_id.into(),
            account_id: account_id.into(),
            access_token: Some("at".into()),
            refresh_token: Some("rt".into()),
            id_token: None,
            access_token_expires_at: Some(9999999999),
            refresh_token_expires_at: None,
            scope: Some("email profile".into()),
            password: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn sqlite_upsert_then_find_by_provider() {
        let b = SqliteAccountBackend::in_memory().unwrap();
        b.upsert(&fixture("google", "u1", "sub_x"));
        let got = b.find_by_provider("google", "sub_x").unwrap();
        assert_eq!(got.user_id, "u1");
        assert_eq!(got.refresh_token.as_deref(), Some("rt"));
    }

    #[test]
    fn sqlite_find_for_user_lists_multiple_providers() {
        let b = SqliteAccountBackend::in_memory().unwrap();
        b.upsert(&fixture("google", "u1", "g_sub"));
        b.upsert(&fixture("github", "u1", "gh_sub"));
        b.upsert(&fixture("google", "u2", "other"));
        let mine = b.find_for_user("u1");
        assert_eq!(mine.len(), 2);
        assert!(mine.iter().any(|a| a.provider_id == "google"));
        assert!(mine.iter().any(|a| a.provider_id == "github"));
    }

    #[test]
    fn sqlite_upsert_is_idempotent_and_refreshes_tokens() {
        let b = SqliteAccountBackend::in_memory().unwrap();
        let mut a = fixture("google", "u1", "sub");
        b.upsert(&a);
        a.access_token = Some("new_at".into());
        a.updated_at = 99;
        b.upsert(&a);
        let got = b.find_by_provider("google", "sub").unwrap();
        assert_eq!(got.access_token.as_deref(), Some("new_at"));
        assert_eq!(got.updated_at, 99);
        assert_eq!(b.find_for_user("u1").len(), 1);
    }

    #[test]
    fn sqlite_unlink_removes_row() {
        let b = SqliteAccountBackend::in_memory().unwrap();
        b.upsert(&fixture("google", "u1", "sub"));
        assert!(b.unlink("google", "sub"));
        assert!(b.find_by_provider("google", "sub").is_none());
        assert!(!b.unlink("google", "sub"), "second unlink is a no-op");
    }

    #[test]
    fn sqlite_password_column_is_present_for_future_credential_provider() {
        // Confirms the column is wired end-to-end so adding email/password
        // auth later doesn't need a schema migration. Forms the basis of
        // the future provider_id="credential" rows.
        let b = SqliteAccountBackend::in_memory().unwrap();
        let mut a = fixture("credential", "u1", "u1");
        a.access_token = None;
        a.refresh_token = None;
        a.password = Some("argon2id$v=19$m=65536,t=3,p=4$...".into());
        b.upsert(&a);
        let got = b.find_by_provider("credential", "u1").unwrap();
        assert_eq!(
            got.password.as_deref(),
            Some("argon2id$v=19$m=65536,t=3,p=4$...")
        );
    }
}
