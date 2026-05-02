//! Persistent audit-log backends. Append-only by API; SQL-level
//! tampering is a separate concern (DB user perms, immudb, S3
//! delivery for archival).
//!
//! Schema is intentionally flat — one row per event, JSON for the
//! metadata bag — so SIEM pipelines (Datadog, Splunk, Loki) can
//! parse with one query.

use std::sync::{Arc, Mutex};

use pylon_auth::audit::{AuditAction, AuditBackend, AuditEvent};
use rusqlite::Connection;

const SQLITE_TABLE: &str = "_pylon_audit_events";
const PG_TABLE: &str = "_pylon_audit_events";

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

pub struct SqliteAuditBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteAuditBackend {
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
                created_at INTEGER NOT NULL,
                action TEXT NOT NULL,
                user_id TEXT,
                actor_id TEXT,
                tenant_id TEXT,
                ip TEXT,
                user_agent TEXT,
                success INTEGER NOT NULL,
                reason TEXT,
                metadata_json TEXT NOT NULL DEFAULT '{{}}'
            );
            CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_tenant_idx
                ON {SQLITE_TABLE}(tenant_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_user_idx
                ON {SQLITE_TABLE}(user_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_actor_idx
                ON {SQLITE_TABLE}(actor_id, created_at DESC);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl AuditBackend for SqliteAuditBackend {
    fn append(&self, e: &AuditEvent) {
        if let Ok(c) = self.conn.lock() {
            let metadata_json = serde_json::to_string(&e.metadata).unwrap_or_else(|_| "{}".into());
            let _ = c.execute(
                &format!(
                    "INSERT INTO {SQLITE_TABLE}
                       (id, created_at, action, user_id, actor_id, tenant_id, ip,
                        user_agent, success, reason, metadata_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
                ),
                rusqlite::params![
                    e.id,
                    e.created_at as i64,
                    e.action.as_str(),
                    e.user_id,
                    e.actor_id,
                    e.tenant_id,
                    e.ip,
                    e.user_agent,
                    if e.success { 1i64 } else { 0 },
                    e.reason,
                    metadata_json,
                ],
            );
        }
    }

    fn find_for_tenant(&self, tenant_id: &str, limit: usize) -> Vec<AuditEvent> {
        let Ok(c) = self.conn.lock() else {
            return vec![];
        };
        // Cap limit at 10k to defeat a runaway query parameter from
        // an admin UI bug or an attacker probing for memory pressure.
        let bounded = limit.min(10_000);
        let mut stmt = match c.prepare(&format!(
            "SELECT id, created_at, action, user_id, actor_id, tenant_id, ip,
                    user_agent, success, reason, metadata_json
             FROM {SQLITE_TABLE}
             WHERE tenant_id = ?1
             ORDER BY created_at DESC
             LIMIT ?2"
        )) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let iter = match stmt.query_map(rusqlite::params![tenant_id, bounded as i64], row_to_event)
        {
            Ok(it) => it,
            Err(_) => return vec![],
        };
        iter.filter_map(|r| r.ok()).collect()
    }

    fn find_for_user(&self, user_id: &str, limit: usize) -> Vec<AuditEvent> {
        let Ok(c) = self.conn.lock() else {
            return vec![];
        };
        let bounded = limit.min(10_000);
        let mut stmt = match c.prepare(&format!(
            "SELECT id, created_at, action, user_id, actor_id, tenant_id, ip,
                    user_agent, success, reason, metadata_json
             FROM {SQLITE_TABLE}
             WHERE user_id = ?1 OR actor_id = ?1
             ORDER BY created_at DESC
             LIMIT ?2"
        )) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let iter = match stmt.query_map(rusqlite::params![user_id, bounded as i64], row_to_event) {
            Ok(it) => it,
            Err(_) => return vec![],
        };
        iter.filter_map(|r| r.ok()).collect()
    }
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEvent> {
    let action_str: String = row.get(2)?;
    let metadata_json: String = row.get(10)?;
    let metadata = serde_json::from_str(&metadata_json).unwrap_or_default();
    let success: i64 = row.get(8)?;
    Ok(AuditEvent {
        id: row.get(0)?,
        created_at: row.get::<_, i64>(1)? as u64,
        action: parse_action(&action_str),
        user_id: row.get(3)?,
        actor_id: row.get(4)?,
        tenant_id: row.get(5)?,
        ip: row.get(6)?,
        user_agent: row.get(7)?,
        success: success != 0,
        reason: row.get(9)?,
        metadata,
    })
}

fn parse_action(s: &str) -> AuditAction {
    match s {
        "sign_in" => AuditAction::SignIn,
        "sign_out" => AuditAction::SignOut,
        "sign_in_failed" => AuditAction::SignInFailed,
        "sign_up" => AuditAction::SignUp,
        "password_change" => AuditAction::PasswordChange,
        "password_reset" => AuditAction::PasswordReset,
        "email_change" => AuditAction::EmailChange,
        "totp_enroll" => AuditAction::TotpEnroll,
        "totp_disable" => AuditAction::TotpDisable,
        "totp_backup_codes_regenerate" => AuditAction::TotpBackupCodesRegenerate,
        "passkey_register" => AuditAction::PasskeyRegister,
        "passkey_revoke" => AuditAction::PasskeyRevoke,
        "api_key_create" => AuditAction::ApiKeyCreate,
        "api_key_revoke" => AuditAction::ApiKeyRevoke,
        "oauth_link" => AuditAction::OauthLink,
        "oauth_unlink" => AuditAction::OauthUnlink,
        "org_create" => AuditAction::OrgCreate,
        "org_delete" => AuditAction::OrgDelete,
        "org_invite_send" => AuditAction::OrgInviteSend,
        "org_invite_accept" => AuditAction::OrgInviteAccept,
        "org_member_remove" => AuditAction::OrgMemberRemove,
        "org_role_change" => AuditAction::OrgRoleChange,
        "account_delete" => AuditAction::AccountDelete,
        other => AuditAction::Custom(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

pub use pg::PostgresAuditBackend;

mod pg {
    use super::*;
    use postgres::Client;

    pub struct PostgresAuditBackend {
        client: Mutex<Client>,
    }

    impl PostgresAuditBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client = pylon_storage::postgres::live::connect_pg(url)?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_TABLE} (
                        id TEXT PRIMARY KEY,
                        created_at BIGINT NOT NULL,
                        action TEXT NOT NULL,
                        user_id TEXT,
                        actor_id TEXT,
                        tenant_id TEXT,
                        ip TEXT,
                        user_agent TEXT,
                        success BOOLEAN NOT NULL,
                        reason TEXT,
                        metadata_json TEXT NOT NULL DEFAULT '{{}}'
                    );
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_tenant_idx
                        ON {PG_TABLE}(tenant_id, created_at DESC);
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_user_idx
                        ON {PG_TABLE}(user_id, created_at DESC);
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_actor_idx
                        ON {PG_TABLE}(actor_id, created_at DESC);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl AuditBackend for PostgresAuditBackend {
        fn append(&self, e: &AuditEvent) {
            if let Ok(mut c) = self.client.lock() {
                let metadata_json =
                    serde_json::to_string(&e.metadata).unwrap_or_else(|_| "{}".into());
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {PG_TABLE}
                           (id, created_at, action, user_id, actor_id, tenant_id, ip,
                            user_agent, success, reason, metadata_json)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
                    ),
                    &[
                        &e.id,
                        &(e.created_at as i64),
                        &e.action.as_str(),
                        &e.user_id,
                        &e.actor_id,
                        &e.tenant_id,
                        &e.ip,
                        &e.user_agent,
                        &e.success,
                        &e.reason,
                        &metadata_json,
                    ],
                );
            }
        }

        fn find_for_tenant(&self, tenant_id: &str, limit: usize) -> Vec<AuditEvent> {
            let Ok(mut c) = self.client.lock() else {
                return vec![];
            };
            let bounded = limit.min(10_000) as i64;
            let rows = match c.query(
                &format!(
                    "SELECT id, created_at, action, user_id, actor_id, tenant_id, ip,
                            user_agent, success, reason, metadata_json
                     FROM {PG_TABLE}
                     WHERE tenant_id = $1
                     ORDER BY created_at DESC
                     LIMIT $2"
                ),
                &[&tenant_id, &bounded],
            ) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            rows.iter().map(pg_row_to_event).collect()
        }

        fn find_for_user(&self, user_id: &str, limit: usize) -> Vec<AuditEvent> {
            let Ok(mut c) = self.client.lock() else {
                return vec![];
            };
            let bounded = limit.min(10_000) as i64;
            let rows = match c.query(
                &format!(
                    "SELECT id, created_at, action, user_id, actor_id, tenant_id, ip,
                            user_agent, success, reason, metadata_json
                     FROM {PG_TABLE}
                     WHERE user_id = $1 OR actor_id = $1
                     ORDER BY created_at DESC
                     LIMIT $2"
                ),
                &[&user_id, &bounded],
            ) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            rows.iter().map(pg_row_to_event).collect()
        }
    }

    fn pg_row_to_event(row: &postgres::Row) -> AuditEvent {
        let action_str: String = row.get(2);
        let metadata_json: String = row.get(10);
        let metadata = serde_json::from_str(&metadata_json).unwrap_or_default();
        AuditEvent {
            id: row.get(0),
            created_at: row.get::<_, i64>(1) as u64,
            action: parse_action(&action_str),
            user_id: row.get(3),
            actor_id: row.get(4),
            tenant_id: row.get(5),
            ip: row.get(6),
            user_agent: row.get(7),
            success: row.get(8),
            reason: row.get(9),
            metadata,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_auth::audit::{AuditAction, AuditEventBuilder};

    #[test]
    fn sqlite_round_trip() {
        let b = SqliteAuditBackend::in_memory().unwrap();
        let e = AuditEventBuilder::new(AuditAction::SignIn)
            .user("u1")
            .tenant("t1")
            .ip("1.2.3.4")
            .user_agent("Test/1.0")
            .meta("method", "password")
            .build();
        b.append(&e);
        let got = b.find_for_user("u1", 10);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].user_id.as_deref(), Some("u1"));
        assert_eq!(got[0].metadata.get("method").map(|s| s.as_str()), Some("password"));
        assert!(got[0].success);
    }

    #[test]
    fn sqlite_tenant_isolation() {
        let b = SqliteAuditBackend::in_memory().unwrap();
        b.append(&AuditEventBuilder::new(AuditAction::SignIn).tenant("a").user("u1").build());
        b.append(&AuditEventBuilder::new(AuditAction::SignIn).tenant("b").user("u2").build());
        assert_eq!(b.find_for_tenant("a", 10).len(), 1);
        assert_eq!(b.find_for_tenant("b", 10).len(), 1);
        assert_eq!(b.find_for_tenant("c", 10).len(), 0);
    }

    #[test]
    fn limit_capped_at_10k() {
        let b = SqliteAuditBackend::in_memory().unwrap();
        // Caller passes a wildly large limit — the cap kicks in
        // before SQL sees it. Here we just check the SQL path
        // doesn't choke on usize::MAX.
        let _ = b.find_for_tenant("t", usize::MAX);
    }

    #[test]
    fn failed_event_persists_with_reason() {
        let b = SqliteAuditBackend::in_memory().unwrap();
        b.append(
            &AuditEventBuilder::new(AuditAction::SignInFailed)
                .user("u1")
                .failed("WRONG_PASSWORD")
                .build(),
        );
        let got = b.find_for_user("u1", 10);
        assert!(!got[0].success);
        assert_eq!(got[0].reason.as_deref(), Some("WRONG_PASSWORD"));
    }

    #[test]
    fn ordering_is_newest_first_via_index() {
        let b = SqliteAuditBackend::in_memory().unwrap();
        // Insert in scrambled time order — the DESC ORDER BY must
        // return them sorted regardless of insertion order.
        for ts in [200u64, 100, 300, 50] {
            b.append(&AuditEvent {
                id: format!("evt_{ts}"),
                created_at: ts,
                action: AuditAction::SignIn,
                user_id: Some("u".into()),
                actor_id: None,
                tenant_id: Some("t".into()),
                ip: None,
                user_agent: None,
                success: true,
                reason: None,
                metadata: std::collections::HashMap::new(),
            });
        }
        let got = b.find_for_tenant("t", 10);
        let timestamps: Vec<u64> = got.iter().map(|e| e.created_at).collect();
        assert_eq!(timestamps, vec![300, 200, 100, 50]);
    }
}
