//! Persistent backends for `pylon_auth::org_sso::OrgSsoStore`. Two
//! tables per backend:
//!
//! - `_pylon_org_sso` — one row per org with the IdP config + cached
//!   discovery endpoints.
//! - `_pylon_org_sso_state` — short-lived state tokens (10-minute TTL,
//!   GC'd opportunistically).
//!
//! Email domains are stored as a JSON array on the config row to keep
//! the schema flat — domain → org lookup walks the row set on demand
//! since the list per server is small (configured orgs typically
//! < 1000).

use std::sync::{Arc, Mutex};

use pylon_auth::org_sso::{OrgSsoConfig, OrgSsoStateRecord, OrgSsoStore, STATE_TTL_SECS};
use rusqlite::Connection;

const SQLITE_CONFIG: &str = "_pylon_org_sso";
const SQLITE_STATE: &str = "_pylon_org_sso_state";
const PG_CONFIG: &str = "_pylon_org_sso";
const PG_STATE: &str = "_pylon_org_sso_state";

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

pub struct SqliteOrgSsoBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteOrgSsoBackend {
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
            "CREATE TABLE IF NOT EXISTS {SQLITE_CONFIG} (
                org_id TEXT PRIMARY KEY,
                issuer_url TEXT NOT NULL,
                client_id TEXT NOT NULL,
                client_secret_sealed TEXT NOT NULL,
                default_role TEXT NOT NULL,
                email_domains_json TEXT NOT NULL DEFAULT '[]',
                authorization_endpoint TEXT NOT NULL,
                token_endpoint TEXT NOT NULL,
                userinfo_endpoint TEXT NOT NULL,
                jwks_uri TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {SQLITE_STATE} (
                state TEXT PRIMARY KEY,
                org_id TEXT NOT NULL,
                pkce_verifier TEXT NOT NULL,
                callback_url TEXT NOT NULL,
                error_callback_url TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS {SQLITE_STATE}_created_idx
                ON {SQLITE_STATE}(created_at);"
        ))
        .map_err(|e| format!("init schema: {e}"))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl OrgSsoStore for SqliteOrgSsoBackend {
    fn get(&self, org_id: &str) -> Option<OrgSsoConfig> {
        let c = self.conn.lock().ok()?;
        let mut stmt = c
            .prepare(&format!(
                "SELECT org_id, issuer_url, client_id, client_secret_sealed,
                        default_role, email_domains_json, authorization_endpoint,
                        token_endpoint, userinfo_endpoint, jwks_uri,
                        created_at, updated_at
                 FROM {SQLITE_CONFIG} WHERE org_id = ?1"
            ))
            .ok()?;
        stmt.query_row(rusqlite::params![org_id], row_to_config)
            .ok()
    }

    fn upsert(&self, config: OrgSsoConfig) {
        if let Ok(c) = self.conn.lock() {
            let domains =
                serde_json::to_string(&config.email_domains).unwrap_or_else(|_| "[]".into());
            let _ = c.execute(
                &format!(
                    "INSERT INTO {SQLITE_CONFIG}
                       (org_id, issuer_url, client_id, client_secret_sealed,
                        default_role, email_domains_json, authorization_endpoint,
                        token_endpoint, userinfo_endpoint, jwks_uri,
                        created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT(org_id) DO UPDATE SET
                       issuer_url = excluded.issuer_url,
                       client_id = excluded.client_id,
                       client_secret_sealed = excluded.client_secret_sealed,
                       default_role = excluded.default_role,
                       email_domains_json = excluded.email_domains_json,
                       authorization_endpoint = excluded.authorization_endpoint,
                       token_endpoint = excluded.token_endpoint,
                       userinfo_endpoint = excluded.userinfo_endpoint,
                       jwks_uri = excluded.jwks_uri,
                       updated_at = excluded.updated_at"
                ),
                rusqlite::params![
                    config.org_id,
                    config.issuer_url,
                    config.client_id,
                    config.client_secret_sealed,
                    config.default_role,
                    domains,
                    config.authorization_endpoint,
                    config.token_endpoint,
                    config.userinfo_endpoint,
                    config.jwks_uri,
                    config.created_at as i64,
                    config.updated_at as i64,
                ],
            );
        }
    }

    fn delete(&self, org_id: &str) -> bool {
        let Ok(c) = self.conn.lock() else {
            return false;
        };
        c.execute(
            &format!("DELETE FROM {SQLITE_CONFIG} WHERE org_id = ?1"),
            rusqlite::params![org_id],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    fn find_by_email_domain(&self, domain: &str) -> Option<String> {
        let c = self.conn.lock().ok()?;
        let lower = domain.to_ascii_lowercase();
        let mut stmt = c
            .prepare(&format!(
                "SELECT org_id, email_domains_json FROM {SQLITE_CONFIG}"
            ))
            .ok()?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .ok()?;
        for row in rows.flatten() {
            let domains: Vec<String> = serde_json::from_str(&row.1).unwrap_or_default();
            if domains.iter().any(|d| d.eq_ignore_ascii_case(&lower)) {
                return Some(row.0);
            }
        }
        None
    }

    fn save_state(&self, record: OrgSsoStateRecord) {
        if let Ok(c) = self.conn.lock() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let _ = c.execute(
                &format!("DELETE FROM {SQLITE_STATE} WHERE created_at < ?1"),
                rusqlite::params![now - STATE_TTL_SECS as i64],
            );
            let _ = c.execute(
                &format!(
                    "INSERT INTO {SQLITE_STATE}
                       (state, org_id, pkce_verifier, callback_url, error_callback_url, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(state) DO NOTHING"
                ),
                rusqlite::params![
                    record.state,
                    record.org_id,
                    record.pkce_verifier,
                    record.callback_url,
                    record.error_callback_url,
                    record.created_at as i64,
                ],
            );
        }
    }

    fn take_state(&self, state: &str, expected_org_id: &str) -> Option<OrgSsoStateRecord> {
        let c = self.conn.lock().ok()?;
        let mut stmt = c
            .prepare(&format!(
                "SELECT state, org_id, pkce_verifier, callback_url, error_callback_url, created_at
                 FROM {SQLITE_STATE} WHERE state = ?1"
            ))
            .ok()?;
        let record: OrgSsoStateRecord = stmt
            .query_row(rusqlite::params![state], state_row_to_record)
            .ok()?;
        if record.org_id != expected_org_id {
            // Cross-org replay attempt: leave the row in place (so the
            // legitimate flow still works) and return None.
            return None;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now.saturating_sub(record.created_at) >= STATE_TTL_SECS {
            let _ = c.execute(
                &format!("DELETE FROM {SQLITE_STATE} WHERE state = ?1"),
                rusqlite::params![state],
            );
            return None;
        }
        let _ = c.execute(
            &format!("DELETE FROM {SQLITE_STATE} WHERE state = ?1"),
            rusqlite::params![state],
        );
        Some(record)
    }
}

fn row_to_config(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrgSsoConfig> {
    let domains_json: String = row.get(5)?;
    let email_domains: Vec<String> = serde_json::from_str(&domains_json).unwrap_or_default();
    Ok(OrgSsoConfig {
        org_id: row.get(0)?,
        issuer_url: row.get(1)?,
        client_id: row.get(2)?,
        client_secret_sealed: row.get(3)?,
        default_role: row.get(4)?,
        email_domains,
        authorization_endpoint: row.get(6)?,
        token_endpoint: row.get(7)?,
        userinfo_endpoint: row.get(8)?,
        jwks_uri: row.get(9)?,
        created_at: row.get::<_, i64>(10)? as u64,
        updated_at: row.get::<_, i64>(11)? as u64,
    })
}

fn state_row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrgSsoStateRecord> {
    Ok(OrgSsoStateRecord {
        state: row.get(0)?,
        org_id: row.get(1)?,
        pkce_verifier: row.get(2)?,
        callback_url: row.get(3)?,
        error_callback_url: row.get(4)?,
        created_at: row.get::<_, i64>(5)? as u64,
    })
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

pub use pg::PostgresOrgSsoBackend;

mod pg {
    use super::*;
    use postgres::Client;

    pub struct PostgresOrgSsoBackend {
        client: Mutex<Client>,
    }

    impl PostgresOrgSsoBackend {
        pub fn connect(url: &str) -> Result<Self, String> {
            let mut client = pylon_storage::postgres::live::connect_pg(url)?;
            client
                .batch_execute(&format!(
                    "CREATE TABLE IF NOT EXISTS {PG_CONFIG} (
                        org_id TEXT PRIMARY KEY,
                        issuer_url TEXT NOT NULL,
                        client_id TEXT NOT NULL,
                        client_secret_sealed TEXT NOT NULL,
                        default_role TEXT NOT NULL,
                        email_domains_json TEXT NOT NULL DEFAULT '[]',
                        authorization_endpoint TEXT NOT NULL,
                        token_endpoint TEXT NOT NULL,
                        userinfo_endpoint TEXT NOT NULL,
                        jwks_uri TEXT NOT NULL,
                        created_at BIGINT NOT NULL,
                        updated_at BIGINT NOT NULL
                    );
                    CREATE TABLE IF NOT EXISTS {PG_STATE} (
                        state TEXT PRIMARY KEY,
                        org_id TEXT NOT NULL,
                        pkce_verifier TEXT NOT NULL,
                        callback_url TEXT NOT NULL,
                        error_callback_url TEXT NOT NULL,
                        created_at BIGINT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS {PG_STATE}_created_idx
                        ON {PG_STATE}(created_at);"
                ))
                .map_err(|e| format!("PG init schema: {e}"))?;
            Ok(Self {
                client: Mutex::new(client),
            })
        }
    }

    impl OrgSsoStore for PostgresOrgSsoBackend {
        fn get(&self, org_id: &str) -> Option<OrgSsoConfig> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT org_id, issuer_url, client_id, client_secret_sealed,
                                default_role, email_domains_json, authorization_endpoint,
                                token_endpoint, userinfo_endpoint, jwks_uri,
                                created_at, updated_at
                         FROM {PG_CONFIG} WHERE org_id = $1"
                    ),
                    &[&org_id],
                )
                .ok()??;
            Some(pg_row_to_config(&row))
        }

        fn upsert(&self, config: OrgSsoConfig) {
            if let Ok(mut c) = self.client.lock() {
                let domains =
                    serde_json::to_string(&config.email_domains).unwrap_or_else(|_| "[]".into());
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {PG_CONFIG}
                           (org_id, issuer_url, client_id, client_secret_sealed,
                            default_role, email_domains_json, authorization_endpoint,
                            token_endpoint, userinfo_endpoint, jwks_uri,
                            created_at, updated_at)
                         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
                         ON CONFLICT(org_id) DO UPDATE SET
                           issuer_url = EXCLUDED.issuer_url,
                           client_id = EXCLUDED.client_id,
                           client_secret_sealed = EXCLUDED.client_secret_sealed,
                           default_role = EXCLUDED.default_role,
                           email_domains_json = EXCLUDED.email_domains_json,
                           authorization_endpoint = EXCLUDED.authorization_endpoint,
                           token_endpoint = EXCLUDED.token_endpoint,
                           userinfo_endpoint = EXCLUDED.userinfo_endpoint,
                           jwks_uri = EXCLUDED.jwks_uri,
                           updated_at = EXCLUDED.updated_at"
                    ),
                    &[
                        &config.org_id,
                        &config.issuer_url,
                        &config.client_id,
                        &config.client_secret_sealed,
                        &config.default_role,
                        &domains,
                        &config.authorization_endpoint,
                        &config.token_endpoint,
                        &config.userinfo_endpoint,
                        &config.jwks_uri,
                        &(config.created_at as i64),
                        &(config.updated_at as i64),
                    ],
                );
            }
        }

        fn delete(&self, org_id: &str) -> bool {
            let Ok(mut c) = self.client.lock() else {
                return false;
            };
            c.execute(
                &format!("DELETE FROM {PG_CONFIG} WHERE org_id = $1"),
                &[&org_id],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
        }

        fn find_by_email_domain(&self, domain: &str) -> Option<String> {
            let mut c = self.client.lock().ok()?;
            let lower = domain.to_ascii_lowercase();
            let rows = c
                .query(
                    &format!("SELECT org_id, email_domains_json FROM {PG_CONFIG}"),
                    &[],
                )
                .ok()?;
            for r in rows {
                let domains_json: String = r.get(1);
                let domains: Vec<String> = serde_json::from_str(&domains_json).unwrap_or_default();
                if domains.iter().any(|d| d.eq_ignore_ascii_case(&lower)) {
                    let oid: String = r.get(0);
                    return Some(oid);
                }
            }
            None
        }

        fn save_state(&self, record: OrgSsoStateRecord) {
            if let Ok(mut c) = self.client.lock() {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let _ = c.execute(
                    &format!("DELETE FROM {PG_STATE} WHERE created_at < $1"),
                    &[&(now - STATE_TTL_SECS as i64)],
                );
                let _ = c.execute(
                    &format!(
                        "INSERT INTO {PG_STATE}
                           (state, org_id, pkce_verifier, callback_url, error_callback_url, created_at)
                         VALUES ($1,$2,$3,$4,$5,$6)
                         ON CONFLICT(state) DO NOTHING"
                    ),
                    &[
                        &record.state,
                        &record.org_id,
                        &record.pkce_verifier,
                        &record.callback_url,
                        &record.error_callback_url,
                        &(record.created_at as i64),
                    ],
                );
            }
        }

        fn take_state(&self, state: &str, expected_org_id: &str) -> Option<OrgSsoStateRecord> {
            let mut c = self.client.lock().ok()?;
            let row = c
                .query_opt(
                    &format!(
                        "SELECT state, org_id, pkce_verifier, callback_url, error_callback_url, created_at
                         FROM {PG_STATE} WHERE state = $1"
                    ),
                    &[&state],
                )
                .ok()??;
            let record = pg_state_row_to_record(&row);
            if record.org_id != expected_org_id {
                return None;
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now.saturating_sub(record.created_at) >= STATE_TTL_SECS {
                let _ = c.execute(
                    &format!("DELETE FROM {PG_STATE} WHERE state = $1"),
                    &[&state],
                );
                return None;
            }
            let _ = c.execute(
                &format!("DELETE FROM {PG_STATE} WHERE state = $1"),
                &[&state],
            );
            Some(record)
        }
    }

    fn pg_row_to_config(row: &postgres::Row) -> OrgSsoConfig {
        let domains_json: String = row.get(5);
        let email_domains: Vec<String> = serde_json::from_str(&domains_json).unwrap_or_default();
        OrgSsoConfig {
            org_id: row.get(0),
            issuer_url: row.get(1),
            client_id: row.get(2),
            client_secret_sealed: row.get(3),
            default_role: row.get(4),
            email_domains,
            authorization_endpoint: row.get(6),
            token_endpoint: row.get(7),
            userinfo_endpoint: row.get(8),
            jwks_uri: row.get(9),
            created_at: row.get::<_, i64>(10) as u64,
            updated_at: row.get::<_, i64>(11) as u64,
        }
    }

    fn pg_state_row_to_record(row: &postgres::Row) -> OrgSsoStateRecord {
        OrgSsoStateRecord {
            state: row.get(0),
            org_id: row.get(1),
            pkce_verifier: row.get(2),
            callback_url: row.get(3),
            error_callback_url: row.get(4),
            created_at: row.get::<_, i64>(5) as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SqliteOrgSsoBackend {
        SqliteOrgSsoBackend::in_memory().unwrap()
    }

    fn config(org: &str, domains: Vec<&str>) -> OrgSsoConfig {
        OrgSsoConfig {
            org_id: org.into(),
            issuer_url: "https://acme.okta.com".into(),
            client_id: "client_abc".into(),
            client_secret_sealed: "plain:shh".into(),
            default_role: "Member".into(),
            email_domains: domains.into_iter().map(String::from).collect(),
            authorization_endpoint: "https://acme.okta.com/oauth2/v1/authorize".into(),
            token_endpoint: "https://acme.okta.com/oauth2/v1/token".into(),
            userinfo_endpoint: "https://acme.okta.com/oauth2/v1/userinfo".into(),
            jwks_uri: "https://acme.okta.com/oauth2/v1/keys".into(),
            created_at: 100,
            updated_at: 100,
        }
    }

    #[test]
    fn round_trip_upsert_get() {
        let s = store();
        s.upsert(config("acme", vec!["acme.com"]));
        let got = s.get("acme").unwrap();
        assert_eq!(got.client_id, "client_abc");
        assert_eq!(got.email_domains, vec!["acme.com".to_string()]);
    }

    #[test]
    fn upsert_replaces_on_conflict() {
        let s = store();
        s.upsert(config("acme", vec!["old.com"]));
        let mut updated = config("acme", vec!["new.com"]);
        updated.client_id = "client_xyz".into();
        updated.updated_at = 200;
        s.upsert(updated);
        let got = s.get("acme").unwrap();
        assert_eq!(got.client_id, "client_xyz");
        assert_eq!(got.email_domains, vec!["new.com".to_string()]);
        assert_eq!(got.updated_at, 200);
    }

    #[test]
    fn find_by_email_domain_walks_rows() {
        let s = store();
        s.upsert(config("acme", vec!["acme.com", "acme.io"]));
        s.upsert(config("globex", vec!["globex.com"]));
        assert_eq!(s.find_by_email_domain("ACME.IO").as_deref(), Some("acme"));
        assert_eq!(
            s.find_by_email_domain("globex.com").as_deref(),
            Some("globex")
        );
        assert_eq!(s.find_by_email_domain("nobody.com"), None);
    }

    #[test]
    fn delete_removes_row() {
        let s = store();
        s.upsert(config("acme", vec![]));
        assert!(s.delete("acme"));
        assert!(s.get("acme").is_none());
        assert!(!s.delete("acme"));
    }

    #[test]
    fn state_round_trip_and_replay_blocked() {
        let s = store();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let rec = OrgSsoStateRecord {
            state: "tok_1".into(),
            org_id: "acme".into(),
            pkce_verifier: "v".into(),
            callback_url: "https://app/cb".into(),
            error_callback_url: "https://app/err".into(),
            created_at: now,
        };
        s.save_state(rec);
        let got = s.take_state("tok_1", "acme").unwrap();
        assert_eq!(got.pkce_verifier, "v");
        // Single-use: second take returns None.
        assert!(s.take_state("tok_1", "acme").is_none());
    }

    #[test]
    fn state_take_rejects_wrong_org() {
        let s = store();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        s.save_state(OrgSsoStateRecord {
            state: "tok_2".into(),
            org_id: "acme".into(),
            pkce_verifier: "v".into(),
            callback_url: "u".into(),
            error_callback_url: "u".into(),
            created_at: now,
        });
        assert!(s.take_state("tok_2", "evil").is_none());
        // Legit org can still consume.
        assert!(s.take_state("tok_2", "acme").is_some());
    }
}
