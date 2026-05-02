//! Persistent verification-token stores for password reset / email
//! change / magic links. Schema is identical to the in-memory shape;
//! `token_prefix` is indexed so consume-by-plaintext is fast.

use std::sync::{Arc, Mutex};

use pylon_auth::verification::{TokenKind, VerificationBackend, VerificationToken};
use rusqlite::Connection;

const SQLITE_TABLE: &str = "_pylon_verification_tokens";
const PG_TABLE: &str = "_pylon_verification_tokens";

fn kind_to_str(k: TokenKind) -> &'static str {
    k.as_str()
}

/// Parse a kind value from the DB. Returns `Err` for unknown values
/// rather than silently defaulting — Wave-6 codex P3: a corrupted
/// row shouldn't be silently re-categorized as a magic-link token,
/// because that would let a stale password-reset row bypass its
/// kind check.
fn kind_from_str(s: &str) -> Result<TokenKind, String> {
    match s {
        "password_reset" => Ok(TokenKind::PasswordReset),
        "email_change" => Ok(TokenKind::EmailChange),
        "magic_link" => Ok(TokenKind::MagicLink),
        other => Err(format!("verification: unknown kind '{other}'")),
    }
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

pub struct SqliteVerificationBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteVerificationBackend {
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
            "CREATE TABLE IF NOT EXISTS {SQLITE_TABLE} (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                email TEXT NOT NULL,
                user_id TEXT,
                payload TEXT,
                token_hash TEXT NOT NULL,
                token_prefix TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                consumed_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_prefix_idx ON {SQLITE_TABLE}(token_prefix);
            CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_exp_idx ON {SQLITE_TABLE}(expires_at);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl VerificationBackend for SqliteVerificationBackend {
    fn put(&self, t: &VerificationToken) {
        if let Ok(c) = self.conn.lock() {
            let _ = c.execute(
                &format!(
                    "INSERT INTO {SQLITE_TABLE}
                       (id, kind, email, user_id, payload, token_hash, token_prefix,
                        created_at, expires_at, consumed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(id) DO UPDATE SET
                       consumed_at = excluded.consumed_at"
                ),
                rusqlite::params![
                    t.id,
                    kind_to_str(t.kind),
                    t.email,
                    t.user_id,
                    t.payload,
                    t.token_hash,
                    t.token_prefix,
                    t.created_at as i64,
                    t.expires_at as i64,
                    t.consumed_at.map(|v| v as i64),
                ],
            );
        }
    }

    fn get(&self, id: &str) -> Option<VerificationToken> {
        let c = self.conn.lock().ok()?;
        c.query_row(
            &format!(
                "SELECT id, kind, email, user_id, payload, token_hash, token_prefix,
                        created_at, expires_at, consumed_at
                 FROM {SQLITE_TABLE} WHERE id = ?1"
            ),
            rusqlite::params![id],
            row_to_token,
        )
        .ok()
    }

    fn by_prefix(&self, prefix: &str) -> Vec<VerificationToken> {
        let Ok(c) = self.conn.lock() else {
            return vec![];
        };
        let mut stmt = match c.prepare(&format!(
            "SELECT id, kind, email, user_id, payload, token_hash, token_prefix,
                    created_at, expires_at, consumed_at
             FROM {SQLITE_TABLE} WHERE token_prefix = ?1"
        )) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let iter = match stmt.query_map(rusqlite::params![prefix], row_to_token) {
            Ok(it) => it,
            Err(_) => return vec![],
        };
        iter.filter_map(|r| r.ok()).collect()
    }

    fn mark_consumed(&self, id: &str, now: u64) -> bool {
        let Ok(c) = self.conn.lock() else {
            return false;
        };
        // CAS via SQL: only update when consumed_at IS NULL. The
        // affected-row count tells us whether we won the race.
        c.execute(
            &format!(
                "UPDATE {SQLITE_TABLE} SET consumed_at = ?2
                 WHERE id = ?1 AND consumed_at IS NULL"
            ),
            rusqlite::params![id, now as i64],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    fn purge_expired(&self, now: u64) {
        if let Ok(c) = self.conn.lock() {
            let _ = c.execute(
                &format!(
                    "DELETE FROM {SQLITE_TABLE}
                     WHERE expires_at <= ?1 AND consumed_at IS NOT NULL"
                ),
                rusqlite::params![now as i64],
            );
        }
    }
}

fn row_to_token(row: &rusqlite::Row<'_>) -> rusqlite::Result<VerificationToken> {
    let kind_raw: String = row.get(1)?;
    let kind = kind_from_str(&kind_raw).map_err(|e| {
        rusqlite::Error::InvalidColumnType(
            1,
            e,
            rusqlite::types::Type::Text,
        )
    })?;
    Ok(VerificationToken {
        id: row.get(0)?,
        kind,
        email: row.get(2)?,
        user_id: row.get(3)?,
        payload: row.get(4)?,
        token_hash: row.get(5)?,
        token_prefix: row.get(6)?,
        created_at: row.get::<_, i64>(7)? as u64,
        expires_at: row.get::<_, i64>(8)? as u64,
        consumed_at: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
    })
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

pub use pg::PostgresVerificationBackend;

mod pg {
    use super::*;
    use postgres::Client;

    pub struct PostgresVerificationBackend {
        client: Mutex<Client>,
    }

    impl PostgresVerificationBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client = pylon_storage::postgres::live::connect_pg(url)?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_TABLE} (
                        id TEXT PRIMARY KEY,
                        kind TEXT NOT NULL,
                        email TEXT NOT NULL,
                        user_id TEXT,
                        payload TEXT,
                        token_hash TEXT NOT NULL,
                        token_prefix TEXT NOT NULL,
                        created_at BIGINT NOT NULL,
                        expires_at BIGINT NOT NULL,
                        consumed_at BIGINT
                    );
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_prefix_idx ON {PG_TABLE}(token_prefix);
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_exp_idx ON {PG_TABLE}(expires_at);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl VerificationBackend for PostgresVerificationBackend {
        fn put(&self, t: &VerificationToken) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {PG_TABLE}
                           (id, kind, email, user_id, payload, token_hash, token_prefix,
                            created_at, expires_at, consumed_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                         ON CONFLICT (id) DO UPDATE SET consumed_at = EXCLUDED.consumed_at"
                    ),
                    &[
                        &t.id,
                        &kind_to_str(t.kind),
                        &t.email,
                        &t.user_id,
                        &t.payload,
                        &t.token_hash,
                        &t.token_prefix,
                        &(t.created_at as i64),
                        &(t.expires_at as i64),
                        &t.consumed_at.map(|v| v as i64),
                    ],
                );
            }
        }

        fn get(&self, id: &str) -> Option<VerificationToken> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT id, kind, email, user_id, payload, token_hash, token_prefix,
                                created_at, expires_at, consumed_at
                         FROM {PG_TABLE} WHERE id = $1"
                    ),
                    &[&id],
                )
                .ok()??;
            pg_row_to_token(&row)
        }

        fn by_prefix(&self, prefix: &str) -> Vec<VerificationToken> {
            let Ok(mut c) = self.client.lock() else {
                return vec![];
            };
            let rows = match c.query(
                &format!(
                    "SELECT id, kind, email, user_id, payload, token_hash, token_prefix,
                            created_at, expires_at, consumed_at
                     FROM {PG_TABLE} WHERE token_prefix = $1"
                ),
                &[&prefix],
            ) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            rows.iter().filter_map(pg_row_to_token).collect()
        }

        fn mark_consumed(&self, id: &str, now: u64) -> bool {
            let Ok(mut c) = self.client.lock() else {
                return false;
            };
            c.execute(
                &format!(
                    "UPDATE {PG_TABLE} SET consumed_at = $2
                     WHERE id = $1 AND consumed_at IS NULL"
                ),
                &[&id, &(now as i64)],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
        }

        fn purge_expired(&self, now: u64) {
            if let Ok(mut c) = self.client.lock() {
                let _ = c.execute(
                    &format!(
                        "DELETE FROM {PG_TABLE}
                         WHERE expires_at <= $1 AND consumed_at IS NOT NULL"
                    ),
                    &[&(now as i64)],
                );
            }
        }
    }

    fn pg_row_to_token(row: &postgres::Row) -> Option<VerificationToken> {
        let kind_raw: String = row.get(1);
        let kind = kind_from_str(&kind_raw).ok()?;
        Some(VerificationToken {
            id: row.get(0),
            kind,
            email: row.get(2),
            user_id: row.get(3),
            payload: row.get(4),
            token_hash: row.get(5),
            token_prefix: row.get(6),
            created_at: row.get::<_, i64>(7) as u64,
            expires_at: row.get::<_, i64>(8) as u64,
            consumed_at: row.get::<_, Option<i64>>(9).map(|v| v as u64),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_auth::verification::{TokenKind, VerificationToken};

    #[test]
    fn sqlite_round_trip() {
        let b = SqliteVerificationBackend::in_memory().unwrap();
        let t = VerificationToken {
            id: "vt_x".into(),
            kind: TokenKind::PasswordReset,
            email: "a@b.com".into(),
            user_id: None,
            payload: None,
            token_hash: "h".into(),
            token_prefix: "abcd1234".into(),
            created_at: 100,
            expires_at: 9_999_999_999,
            consumed_at: None,
        };
        b.put(&t);
        assert_eq!(b.get("vt_x").unwrap().email, "a@b.com");
        assert_eq!(b.by_prefix("abcd1234").len(), 1);
        assert_eq!(b.by_prefix("nope0000").len(), 0);
        // mark_consumed CAS
        assert!(b.mark_consumed("vt_x", 200));
        assert!(!b.mark_consumed("vt_x", 300)); // second attempt loses
        assert_eq!(b.get("vt_x").unwrap().consumed_at, Some(200));
    }

    #[test]
    fn purge_drops_expired_consumed() {
        let b = SqliteVerificationBackend::in_memory().unwrap();
        b.put(&VerificationToken {
            id: "vt_done".into(),
            kind: TokenKind::MagicLink,
            email: "a@b.com".into(),
            user_id: None,
            payload: None,
            token_hash: "h".into(),
            token_prefix: "p".into(),
            created_at: 1,
            expires_at: 2,
            consumed_at: Some(2),
        });
        b.purge_expired(100);
        assert!(b.get("vt_done").is_none());
    }
}
