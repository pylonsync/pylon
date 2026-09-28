//! Persistent audit-log backends. Append-only by API; SQL-level
//! tampering is a separate concern (DB user perms, immudb, S3
//! delivery for archival).
//!
//! Schema is intentionally flat — one row per event, JSON for the
//! metadata bag — so SIEM pipelines (Datadog, Splunk, Loki) can
//! parse with one query.

use std::sync::{Arc, Mutex};

use pylon_auth::audit::{AuditAction, AuditBackend, AuditEvent, AuditQuery};

const COLUMNS: &str = "id, created_at, action, user_id, actor_id, tenant_id, ip, \
     user_agent, success, reason, metadata_json, entity, entity_id";
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
        // Columns added for application events. SQLite has no
        // ADD COLUMN IF NOT EXISTS, so check table_info first.
        let existing: std::collections::HashSet<String> = conn
            .prepare(&format!(
                "SELECT name FROM pragma_table_info('{SQLITE_TABLE}')"
            ))
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<_, _>>()
            })
            .map_err(|e| format!("inspect schema: {e}"))?;
        for column in ["entity", "entity_id"] {
            if !existing.contains(column) {
                conn.execute_batch(&format!(
                    "ALTER TABLE {SQLITE_TABLE} ADD COLUMN {column} TEXT"
                ))
                .map_err(|e| format!("migrate schema: {e}"))?;
            }
        }
        conn.execute_batch(&format!(
            "CREATE INDEX IF NOT EXISTS {SQLITE_TABLE}_entity_idx
                ON {SQLITE_TABLE}(entity, entity_id, created_at DESC);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl AuditBackend for SqliteAuditBackend {
    fn try_append(&self, e: &AuditEvent) -> Result<(), String> {
        let c = self
            .conn
            .lock()
            .map_err(|_| "audit store lock poisoned".to_string())?;
        let metadata_json = serde_json::to_string(&e.metadata).unwrap_or_else(|_| "{}".into());
        c.execute(
            &format!(
                "INSERT INTO {SQLITE_TABLE} ({COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
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
                e.entity,
                e.entity_id,
            ],
        )
        .map(|_| ())
        .map_err(|e| format!("audit insert failed: {e}"))
    }

    fn find(&self, q: &AuditQuery) -> Result<Vec<AuditEvent>, String> {
        let c = self
            .conn
            .lock()
            .map_err(|_| "audit store lock poisoned".to_string())?;
        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for (column, value) in [
            ("tenant_id", &q.tenant_id),
            ("actor_id", &q.actor_id),
            ("user_id", &q.user_id),
            ("entity", &q.entity),
            ("entity_id", &q.entity_id),
            ("action", &q.action),
        ] {
            if let Some(v) = value {
                params.push(Box::new(v.clone()));
                clauses.push(format!("{column} = ?{}", params.len()));
            }
        }
        if let Some(before) = q.before {
            params.push(Box::new(before.min(i64::MAX as u64) as i64));
            clauses.push(format!("created_at < ?{}", params.len()));
        }
        if let Some(id) = &q.before_id {
            // Strictly older than the cursor event by (created_at, rowid).
            // An unknown id makes both subqueries NULL, so nothing matches.
            params.push(Box::new(id.clone()));
            let i = params.len();
            clauses.push(format!(
                "(created_at, rowid) < \
                 ((SELECT created_at FROM {SQLITE_TABLE} WHERE id = ?{i}), \
                  (SELECT rowid FROM {SQLITE_TABLE} WHERE id = ?{i}))"
            ));
        }
        params.push(Box::new(q.bounded_limit() as i64));
        let limit_idx = params.len();
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let mut stmt = c
            .prepare(&format!(
                "SELECT {COLUMNS} FROM {SQLITE_TABLE} {where_sql}
                 ORDER BY created_at DESC, rowid DESC LIMIT ?{limit_idx}"
            ))
            .map_err(|e| format!("audit query failed: {e}"))?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), row_to_event)
            .map_err(|e| format!("audit query failed: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("audit query failed: {e}"))
    }

    fn find_for_tenant(&self, tenant_id: &str, limit: usize) -> Vec<AuditEvent> {
        let Ok(c) = self.conn.lock() else {
            return vec![];
        };
        // Cap limit at 10k to defeat a runaway query parameter from
        // an admin UI bug or an attacker probing for memory pressure.
        let bounded = limit.min(10_000);
        let mut stmt = match c.prepare(&format!(
            "SELECT {COLUMNS}
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
            "SELECT {COLUMNS}
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
        entity: row.get(11)?,
        entity_id: row.get(12)?,
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
        "anonymous_merge" => AuditAction::AnonymousMerge,
        other => AuditAction::Custom(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

pub use pg::PostgresAuditBackend;

mod pg {
    use super::*;
    use pylon_storage::pg_datastore::PgPool;

    pub struct PostgresAuditBackend {
        conn: std::sync::Arc<PgPool>,
    }

    impl PostgresAuditBackend {
        pub fn with_pool(conn: std::sync::Arc<PgPool>) -> Result<Self, String> {
            conn.with_client(|c| {
                c.batch_execute(&format!(
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
                        ON {PG_TABLE}(actor_id, created_at DESC);
                    ALTER TABLE {PG_TABLE}
                        ADD COLUMN IF NOT EXISTS entity TEXT,
                        ADD COLUMN IF NOT EXISTS entity_id TEXT,
                        ADD COLUMN IF NOT EXISTS seq BIGSERIAL;
                    CREATE INDEX IF NOT EXISTS {PG_TABLE}_entity_idx
                        ON {PG_TABLE}(entity, entity_id, created_at DESC);"
                ))
            })
            .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self { conn })
        }
    }

    impl AuditBackend for PostgresAuditBackend {
        fn try_append(&self, e: &AuditEvent) -> Result<(), String> {
            let metadata_json = serde_json::to_string(&e.metadata).unwrap_or_else(|_| "{}".into());
            self.conn
                .with_client(|c| {
                    c.execute(
                        &format!(
                            "INSERT INTO {PG_TABLE} ({COLUMNS})
                             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"
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
                            &e.entity,
                            &e.entity_id,
                        ],
                    )
                })
                .map(|_| ())
                .map_err(|e| format!("audit insert failed: {e}"))
        }

        fn find(&self, q: &AuditQuery) -> Result<Vec<AuditEvent>, String> {
            let before = q.before.map(|b| b.min(i64::MAX as u64) as i64);
            let limit = q.bounded_limit() as i64;
            let rows = self
                .conn
                .with_client(|c| {
                    c.query(
                        &format!(
                            "SELECT {COLUMNS} FROM {PG_TABLE}
                             WHERE ($1::TEXT IS NULL OR tenant_id = $1)
                               AND ($2::TEXT IS NULL OR actor_id = $2)
                               AND ($3::TEXT IS NULL OR user_id = $3)
                               AND ($4::TEXT IS NULL OR entity = $4)
                               AND ($5::TEXT IS NULL OR entity_id = $5)
                               AND ($6::TEXT IS NULL OR action = $6)
                               AND ($7::BIGINT IS NULL OR created_at < $7)
                             AND ($9::TEXT IS NULL OR (created_at, seq) <
                                  (SELECT created_at, seq FROM {PG_TABLE} WHERE id = $9))
                             ORDER BY created_at DESC, seq DESC
                             LIMIT $8"
                        ),
                        &[
                            &q.tenant_id,
                            &q.actor_id,
                            &q.user_id,
                            &q.entity,
                            &q.entity_id,
                            &q.action,
                            &before,
                            &limit,
                            &q.before_id,
                        ],
                    )
                })
                .map_err(|e| format!("audit query failed: {e}"))?;
            Ok(rows.iter().map(pg_row_to_event).collect())
        }

        fn find_for_tenant(&self, tenant_id: &str, limit: usize) -> Vec<AuditEvent> {
            let bounded = limit.min(10_000) as i64;
            let rows = match self.conn.with_client(|c| {
                c.query(
                    &format!(
                        "SELECT {COLUMNS}
                         FROM {PG_TABLE}
                         WHERE tenant_id = $1
                         ORDER BY created_at DESC
                         LIMIT $2"
                    ),
                    &[&tenant_id, &bounded],
                )
            }) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("[pg] audit find_for_tenant failed: {e}");
                    return vec![];
                }
            };
            rows.iter().map(pg_row_to_event).collect()
        }

        fn find_for_user(&self, user_id: &str, limit: usize) -> Vec<AuditEvent> {
            let bounded = limit.min(10_000) as i64;
            let rows = match self.conn.with_client(|c| {
                c.query(
                    &format!(
                        "SELECT {COLUMNS}
                         FROM {PG_TABLE}
                         WHERE user_id = $1 OR actor_id = $1
                         ORDER BY created_at DESC
                         LIMIT $2"
                    ),
                    &[&user_id, &bounded],
                )
            }) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("[pg] audit find_for_user failed: {e}");
                    return vec![];
                }
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
            entity: row.get(11),
            entity_id: row.get(12),
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
        assert_eq!(
            got[0].metadata.get("method").map(|s| s.as_str()),
            Some("password")
        );
        assert!(got[0].success);
    }

    #[test]
    fn sqlite_tenant_isolation() {
        let b = SqliteAuditBackend::in_memory().unwrap();
        b.append(
            &AuditEventBuilder::new(AuditAction::SignIn)
                .tenant("a")
                .user("u1")
                .build(),
        );
        b.append(
            &AuditEventBuilder::new(AuditAction::SignIn)
                .tenant("b")
                .user("u2")
                .build(),
        );
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
                entity: None,
                entity_id: None,
            });
        }
        let got = b.find_for_tenant("t", 10);
        let timestamps: Vec<u64> = got.iter().map(|e| e.created_at).collect();
        assert_eq!(timestamps, vec![300, 200, 100, 50]);
    }

    fn app_event(action: &str, tenant: &str, actor: &str, entity_id: &str, at: u64) -> AuditEvent {
        let mut e = AuditEventBuilder::new(AuditAction::Custom(action.into()))
            .tenant(tenant)
            .actor(actor)
            .entity("Lead", Some(entity_id.into()))
            .meta("k", "v")
            .build();
        e.created_at = at;
        e
    }

    fn check_find(b: &dyn AuditBackend) {
        let tag = pylon_cluster::new_instance_id();
        let t1 = format!("t1_{tag}");
        let t2 = format!("t2_{tag}");
        for (i, (tenant, actor, lead)) in [(&t1, "a", "l1"), (&t1, "b", "l2"), (&t2, "c", "l1")]
            .into_iter()
            .enumerate()
        {
            b.try_append(&app_event(
                "app.lead.view",
                tenant,
                actor,
                lead,
                1000 + i as u64,
            ))
            .unwrap();
        }
        // Same second: newest append first.
        b.try_append(&app_event("app.lead.export", &t1, "a", "l1", 1002))
            .unwrap();
        b.try_append(&app_event("app.lead.export", &t1, "b", "l1", 1002))
            .unwrap();

        let q = |f: &dyn Fn(&mut AuditQuery)| {
            let mut q = AuditQuery {
                tenant_id: Some(t1.clone()),
                limit: 50,
                ..Default::default()
            };
            f(&mut q);
            b.find(&q).unwrap()
        };
        assert_eq!(q(&|_| {}).len(), 4);
        let exports = q(&|q| q.action = Some("app.lead.export".into()));
        assert_eq!(
            exports
                .iter()
                .map(|e| e.actor_id.clone().unwrap())
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        assert_eq!(q(&|q| q.entity_id = Some("l2".into())).len(), 1);
        assert_eq!(q(&|q| q.before = Some(1002)).len(), 2);
        assert_eq!(q(&|q| q.limit = 1).len(), 1);
        // Paging with the cursor walks every event exactly once, even
        // when a page ends inside one second.
        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = q(&|q| {
                q.limit = 2;
                q.before_id = cursor.clone();
            });
            if page.is_empty() {
                break;
            }
            cursor = page.last().map(|e| e.id.clone());
            seen.extend(page.into_iter().map(|e| e.id));
        }
        let all: Vec<String> = q(&|_| {}).into_iter().map(|e| e.id).collect();
        assert_eq!(seen, all);
        assert!(q(&|q| q.before_id = Some("evt_unknown".into())).is_empty());
        let first = &q(&|q| q.entity_id = Some("l2".into()))[0];
        assert_eq!(first.entity.as_deref(), Some("Lead"));
        assert_eq!(first.metadata.get("k").map(String::as_str), Some("v"));
    }

    #[test]
    fn sqlite_find_filters_and_orders() {
        check_find(&SqliteAuditBackend::in_memory().unwrap());
    }

    #[test]
    fn sqlite_schema_migrates_an_old_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE {SQLITE_TABLE} (
                id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, action TEXT NOT NULL,
                user_id TEXT, actor_id TEXT, tenant_id TEXT, ip TEXT, user_agent TEXT,
                success INTEGER NOT NULL, reason TEXT, metadata_json TEXT NOT NULL DEFAULT '{{}}'
            );"
        ))
        .unwrap();
        let b = SqliteAuditBackend::from_connection(conn).unwrap();
        check_find(&b);
    }

    #[test]
    fn postgres_find_filters_and_orders() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let pool = pylon_storage::pg_datastore::PgPool::connect(
            &url,
            2,
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        check_find(&PostgresAuditBackend::with_pool(pool).unwrap());
    }
}
