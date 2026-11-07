//! Persistent stores for the access and refresh tokens Pylon's OIDC
//! provider issues (`pylon_auth::oidc_tokens`).
//!
//! One row per token, keyed by the SHA-256 hash of the token; the raw
//! token is never stored. `grant_id` ties every token of one sign-in
//! together (indexed: revocation deletes by grant), `user_id` is indexed
//! for "revoke everything of this user", and `expires_at` for the sweep.
//! Timestamps are unix seconds. `rotated_at` is set on a refresh token the
//! first time it is exchanged.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use pylon_auth::oidc_tokens::{
    split_scopes, validate_token_bundle, OidcTokenKind, OidcTokenRecord, OidcTokenStore,
};
use rusqlite::{Connection, OptionalExtension};

const TABLE: &str = "_pylon_oidc_tokens";

const COLUMNS: &str =
    "token_hash, kind, grant_id, user_id, client_id, scopes, expires_at, created_at, rotated_at";

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

pub struct SqliteOidcTokenBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteOidcTokenBackend {
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
                token_hash TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                grant_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                client_id TEXT NOT NULL,
                scopes TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                rotated_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS {TABLE}_grant_idx ON {TABLE}(grant_id);
            CREATE INDEX IF NOT EXISTS {TABLE}_user_idx ON {TABLE}(user_id);
            CREATE INDEX IF NOT EXISTS {TABLE}_exp_idx ON {TABLE}(expires_at);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.conn
            .lock()
            .map_err(|_| "OIDC token store lock poisoned".to_string())
    }
}

fn sqlite_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OidcTokenRecord> {
    let kind: String = row.get(1)?;
    let scopes: String = row.get(5)?;
    Ok(OidcTokenRecord {
        token_hash: row.get(0)?,
        // An unknown kind can only come from a hand-edited row; read it as
        // the kind that grants the least (an access token can't mint
        // anything).
        kind: OidcTokenKind::parse(&kind).unwrap_or(OidcTokenKind::Access),
        grant_id: row.get(2)?,
        user_id: row.get(3)?,
        client_id: row.get(4)?,
        scopes: split_scopes(&scopes),
        expires_at: row.get::<_, i64>(6)? as u64,
        created_at: row.get::<_, i64>(7)? as u64,
        rotated_at: row.get::<_, Option<i64>>(8)?.map(|n| n as u64),
    })
}

impl OidcTokenStore for SqliteOidcTokenBackend {
    fn insert_tokens(
        &self,
        records: &[OidcTokenRecord],
        require_live_grant: bool,
    ) -> Result<(), String> {
        let first = validate_token_bundle(records)?;
        let mut conn = self.lock()?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| format!("begin OIDC issue: {e}"))?;
        if require_live_grant {
            let active: bool = tx
                .query_row(
                    &format!(
                        "SELECT EXISTS(SELECT 1 FROM {TABLE} WHERE grant_id = ?1
                    AND user_id = ?2 AND client_id = ?3 AND kind = 'refresh' AND expires_at > ?4)"
                    ),
                    rusqlite::params![
                        first.grant_id,
                        first.user_id,
                        first.client_id,
                        first.created_at as i64
                    ],
                    |row| row.get(0),
                )
                .map_err(|e| format!("check OIDC grant: {e}"))?;
            if !active {
                return Err("OIDC grant is no longer active".into());
            }
        }
        for r in records {
            tx.execute(
                &format!(
                    "INSERT INTO {TABLE} ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
                ),
                rusqlite::params![
                    r.token_hash,
                    r.kind.as_str(),
                    r.grant_id,
                    r.user_id,
                    r.client_id,
                    r.scopes_joined(),
                    r.expires_at as i64,
                    r.created_at as i64,
                    r.rotated_at.map(|n| n as i64)
                ],
            )
            .map_err(|e| format!("insert OIDC token: {e}"))?;
        }
        tx.commit().map_err(|e| format!("commit OIDC issue: {e}"))
    }

    fn find(&self, token_hash: &str, now: u64) -> Result<Option<OidcTokenRecord>, String> {
        let c = self.lock()?;
        let found = c
            .query_row(
                &format!("SELECT {COLUMNS} FROM {TABLE} WHERE token_hash = ?1"),
                rusqlite::params![token_hash],
                sqlite_row,
            )
            .optional()
            .map_err(|e| format!("find OIDC token: {e}"))?;
        match found {
            Some(r) if r.is_expired(now) => {
                c.execute(
                    &format!("DELETE FROM {TABLE} WHERE token_hash = ?1"),
                    rusqlite::params![token_hash],
                )
                .map_err(|e| format!("delete expired OIDC token: {e}"))?;
                Ok(None)
            }
            other => Ok(other),
        }
    }

    fn mark_rotated(&self, token_hash: &str, now: u64) -> Result<Option<u64>, String> {
        self.lock()?
            .query_row(
                &format!(
                    "UPDATE {TABLE} SET rotated_at = COALESCE(rotated_at, ?2)
                     WHERE token_hash = ?1 RETURNING rotated_at"
                ),
                rusqlite::params![token_hash, now as i64],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|t| t.map(|n| n as u64))
            .map_err(|e| format!("rotate OIDC token: {e}"))
    }

    fn revoke_grant(&self, grant_id: &str) -> Result<usize, String> {
        self.lock()?
            .execute(
                &format!("DELETE FROM {TABLE} WHERE grant_id = ?1"),
                rusqlite::params![grant_id],
            )
            .map_err(|e| format!("revoke OIDC grant: {e}"))
    }

    fn revoke_user(&self, user_id: &str) -> Result<usize, String> {
        let c = self.lock()?;
        let mut stmt = c
            .prepare(&format!(
                "DELETE FROM {TABLE} WHERE user_id = ?1 RETURNING grant_id"
            ))
            .map_err(|e| format!("revoke OIDC user: {e}"))?;
        let grants = stmt
            .query_map(rusqlite::params![user_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("revoke OIDC user: {e}"))?
            .collect::<Result<HashSet<String>, _>>()
            .map_err(|e| format!("revoke OIDC user: {e}"))?;
        Ok(grants.len())
    }

    fn sweep_expired(&self, now: u64) -> Result<usize, String> {
        self.lock()?
            .execute(
                &format!("DELETE FROM {TABLE} WHERE expires_at <= ?1"),
                rusqlite::params![now as i64],
            )
            .map_err(|e| format!("sweep OIDC tokens: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

pub use pg::PostgresOidcTokenBackend;

mod pg {
    use super::*;
    use pylon_storage::pg_datastore::PgPool;

    pub struct PostgresOidcTokenBackend {
        conn: Arc<PgPool>,
    }

    impl PostgresOidcTokenBackend {
        pub fn with_pool(conn: Arc<PgPool>) -> Result<Self, String> {
            conn.with_client(|c| {
                c.batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {TABLE} (
                        token_hash TEXT PRIMARY KEY,
                        kind TEXT NOT NULL,
                        grant_id TEXT NOT NULL,
                        user_id TEXT NOT NULL,
                        client_id TEXT NOT NULL,
                        scopes TEXT NOT NULL,
                        expires_at BIGINT NOT NULL,
                        created_at BIGINT NOT NULL,
                        rotated_at BIGINT
                    );
                    CREATE INDEX IF NOT EXISTS {TABLE}_grant_idx ON {TABLE}(grant_id);
                    CREATE INDEX IF NOT EXISTS {TABLE}_user_idx ON {TABLE}(user_id);
                    CREATE INDEX IF NOT EXISTS {TABLE}_exp_idx ON {TABLE}(expires_at);"
                ))
            })
            .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self { conn })
        }
    }

    fn pg_row(row: &postgres::Row) -> OidcTokenRecord {
        let kind: String = row.get(1);
        let scopes: String = row.get(5);
        OidcTokenRecord {
            token_hash: row.get(0),
            kind: OidcTokenKind::parse(&kind).unwrap_or(OidcTokenKind::Access),
            grant_id: row.get(2),
            user_id: row.get(3),
            client_id: row.get(4),
            scopes: split_scopes(&scopes),
            expires_at: row.get::<_, i64>(6) as u64,
            created_at: row.get::<_, i64>(7) as u64,
            rotated_at: row.get::<_, Option<i64>>(8).map(|n| n as u64),
        }
    }

    // All replicas use this transaction lock for issuance and revocation.
    // Lock by user so grant revocation and user revocation share the same lock.
    // Read committed statements after the lock see the preceding transaction.
    fn lock_user(tx: &mut postgres::Transaction<'_>, user_id: &str) -> Result<(), postgres::Error> {
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('pylon:oidc:' || $1::text, 0))",
            &[&user_id],
        )?;
        Ok(())
    }

    impl OidcTokenStore for PostgresOidcTokenBackend {
        fn insert_tokens(
            &self,
            records: &[OidcTokenRecord],
            require_live_grant: bool,
        ) -> Result<(), String> {
            let first = validate_token_bundle(records)?;
            self.conn.with_client(|c| {
                let mut tx = c.build_transaction()
                        .isolation_level(postgres::IsolationLevel::ReadCommitted).start()?;
                lock_user(&mut tx, &first.user_id)?;
                if require_live_grant {
                    let active: bool = tx.query_one(
                        &format!("SELECT EXISTS(SELECT 1 FROM {TABLE} WHERE grant_id = $1
                            AND user_id = $2 AND client_id = $3 AND kind = 'refresh' AND expires_at > $4)"),
                        &[&first.grant_id, &first.user_id, &first.client_id, &(first.created_at as i64)],
                    )?.get(0);
                    if !active { return Ok(false); }
                }
                for r in records {
                    tx.execute(
                        &format!("INSERT INTO {TABLE} ({COLUMNS}) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"),
                        &[&r.token_hash, &r.kind.as_str(), &r.grant_id, &r.user_id, &r.client_id,
                            &r.scopes_joined(), &(r.expires_at as i64), &(r.created_at as i64),
                            &r.rotated_at.map(|n| n as i64)],
                    )?;
                }
                tx.commit()?;
                Ok(true)
            }).map_err(|e| format!("issue OIDC tokens: {e}"))?
                .then_some(()).ok_or_else(|| "OIDC grant is no longer active".into())
        }

        fn find(&self, token_hash: &str, now: u64) -> Result<Option<OidcTokenRecord>, String> {
            let row = self
                .conn
                .with_client(|c| {
                    c.query_opt(
                        &format!("SELECT {COLUMNS} FROM {TABLE} WHERE token_hash = $1"),
                        &[&token_hash],
                    )
                })
                .map_err(|e| format!("find OIDC token: {e}"))?;
            match row.as_ref().map(pg_row) {
                Some(r) if r.is_expired(now) => {
                    self.conn
                        .with_client(|c| {
                            c.execute(
                                &format!("DELETE FROM {TABLE} WHERE token_hash = $1"),
                                &[&token_hash],
                            )
                        })
                        .map_err(|e| format!("delete expired OIDC token: {e}"))?;
                    Ok(None)
                }
                other => Ok(other),
            }
        }

        fn mark_rotated(&self, token_hash: &str, now: u64) -> Result<Option<u64>, String> {
            // One statement under the row lock: of two concurrent refreshes,
            // exactly one sets the time and both read the same value back.
            let now = now as i64;
            self.conn
                .with_client(|c| {
                    c.query_opt(
                        &format!(
                            "UPDATE {TABLE} SET rotated_at = COALESCE(rotated_at, $2)
                             WHERE token_hash = $1 RETURNING rotated_at"
                        ),
                        &[&token_hash, &now],
                    )
                })
                .map(|row| row.map(|r| r.get::<_, i64>(0) as u64))
                .map_err(|e| format!("rotate OIDC token: {e}"))
        }

        fn revoke_grant(&self, grant_id: &str) -> Result<usize, String> {
            self.conn
                .with_client(|c| {
                    let mut tx = c
                        .build_transaction()
                        .isolation_level(postgres::IsolationLevel::ReadCommitted)
                        .start()?;
                    let owner = tx.query_opt(
                        &format!("SELECT user_id FROM {TABLE} WHERE grant_id = $1 LIMIT 1"),
                        &[&grant_id],
                    )?;
                    let Some(owner) = owner else {
                        return Ok(0);
                    };
                    lock_user(&mut tx, owner.get::<_, &str>(0))?;
                    let removed = tx.execute(
                        &format!("DELETE FROM {TABLE} WHERE grant_id = $1"),
                        &[&grant_id],
                    )?;
                    tx.commit()?;
                    Ok(removed as usize)
                })
                .map_err(|e| format!("revoke OIDC grant: {e}"))
        }

        fn revoke_user(&self, user_id: &str) -> Result<usize, String> {
            self.conn.with_client(|c| {
                let mut tx = c.build_transaction()
                        .isolation_level(postgres::IsolationLevel::ReadCommitted).start()?;
                lock_user(&mut tx, user_id)?;
                let row = tx.query_one(
                    &format!("WITH gone AS (DELETE FROM {TABLE} WHERE user_id = $1 RETURNING grant_id)
                        SELECT COUNT(DISTINCT grant_id) FROM gone"), &[&user_id],
                )?;
                tx.commit()?;
                Ok(row.get::<_, i64>(0) as usize)
            }).map_err(|e| format!("revoke OIDC user: {e}"))
        }

        fn sweep_expired(&self, now: u64) -> Result<usize, String> {
            let now = now as i64;
            self.conn
                .with_client(|c| {
                    c.execute(
                        &format!("DELETE FROM {TABLE} WHERE expires_at <= $1"),
                        &[&now],
                    )
                })
                .map(|n| n as usize)
                .map_err(|e| format!("sweep OIDC tokens: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_auth::oidc_tokens::conformance;

    #[test]
    fn sqlite_store_passes_conformance() {
        conformance::run(&SqliteOidcTokenBackend::in_memory().unwrap(), "sqlite");
    }

    #[test]
    fn sqlite_connections_serialize_refresh_and_revocation() {
        let path = tmp_db();
        let a = SqliteOidcTokenBackend::open(&path).unwrap();
        let b = SqliteOidcTokenBackend::open(&path).unwrap();
        conformance::concurrent_revocation(&a, &b, "sqlite-connections");
        drop((a, b));
        let _ = std::fs::remove_file(path);
    }

    fn tmp_db() -> String {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!(
                "pylon-oidc-tokens-{}-{}-{id}.db",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .to_string_lossy()
            .into_owned()
    }

    fn provider(store: Arc<dyn OidcTokenStore>) -> pylon_auth::oidc_provider::OidcProvider {
        static KEY: std::sync::OnceLock<Arc<pylon_auth::oidc_provider::OidcKeyStore>> =
            std::sync::OnceLock::new();
        let key = KEY
            .get_or_init(|| {
                let path = std::path::PathBuf::from(format!("{}.pem", tmp_db()));
                Arc::new(pylon_auth::oidc_provider::OidcKeyStore::load_or_generate(&path).unwrap())
            })
            .clone();
        pylon_auth::oidc_provider::OidcProvider::new(key, vec![], store)
    }

    /// A restart is a new process: a new provider over a new connection to
    /// the same database file. Tokens issued before it still work, and a
    /// refresh token rotated before it is still caught when reused late.
    #[test]
    fn tokens_survive_a_restart() {
        let path = tmp_db();
        let scopes: Vec<String> = vec!["openid".into(), "offline_access".into()];
        let issued = {
            let before = provider(Arc::new(SqliteOidcTokenBackend::open(&path).unwrap()));
            let issued = before
                .issue_tokens(None, "u1", "c1", &scopes, Some(&scopes))
                .unwrap();
            assert!(before
                .lookup_access_token(&issued.access_token)
                .unwrap()
                .is_some());
            issued
        };

        let after = provider(Arc::new(SqliteOidcTokenBackend::open(&path).unwrap()));
        let access = after
            .lookup_access_token(&issued.access_token)
            .unwrap()
            .expect("access token survives the restart");
        assert_eq!(access.user_id, "u1");
        assert_eq!(access.grant_id, issued.grant_id);
        let accepted = after
            .accept_refresh_token("c1", issued.refresh_token.as_deref().unwrap())
            .expect("refresh token survives the restart");
        assert_eq!(accepted.grant_id, issued.grant_id);

        // Nothing in the file is a raw token.
        let conn = Connection::open(&path).unwrap();
        let raw: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {TABLE} WHERE token_hash = ?1 OR token_hash = ?2"),
                rusqlite::params![
                    issued.access_token,
                    issued.refresh_token.as_deref().unwrap()
                ],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, 0);
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}
