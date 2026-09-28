//! Append-only audit log for security-relevant events.
//!
//! Records the **who, when, what, from where** of every auth state
//! change so SIEM tooling + customer compliance asks can reconstruct
//! the timeline. Designed for ops to trust:
//!
//! - **Append-only by API** — no `update`/`delete` on the trait. SQL
//!   backends can still be tampered with at the DB layer; that's a
//!   separate problem (DB user permissions, immudb, etc.).
//! - **Tenant-scoped queries** — `find_for_user` / `find_for_tenant`
//!   enforce isolation at the store layer so the wrong query
//!   parameters can't accidentally leak cross-tenant events.
//! - **Bounded payload** — events carry a fixed-shape struct, not
//!   arbitrary JSON. Apps that want richer payloads stash structured
//!   metadata in `metadata: Map<String, String>` (string-only values
//!   to keep PII surface predictable).
//!
//! Wire format intentionally short on detail (no full request bodies,
//! no Authorization headers, no passwords). Operators should pair
//! the log with proper request-tracing for debugging.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// One audit-log row. Writes are append-only; the only mutation
/// path is creating a new event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Stable id (`evt_<24-base64url>`).
    pub id: String,
    /// Unix-epoch seconds. Wall-clock from the server's perspective.
    pub created_at: u64,
    /// What happened. Stable enum so SIEM dashboards can match on
    /// well-known names. Apps that need bespoke events use
    /// `AuditAction::Custom("...")`.
    pub action: AuditAction,
    /// User the event is ABOUT (subject). Distinct from `actor_id`
    /// — an admin disabling a user's account has actor=admin,
    /// subject=user.
    pub user_id: Option<String>,
    /// User who PERFORMED the action. Same as `user_id` for self-
    /// service flows. None for system-driven events
    /// (token-refresh tick, scheduled cleanup).
    pub actor_id: Option<String>,
    /// Active org / tenant when the action happened — set when the
    /// caller's session had `tenant_id`.
    pub tenant_id: Option<String>,
    /// Source IP of the request. Apps with a CDN should ensure this
    /// is the REAL client IP (X-Forwarded-For has been parsed).
    pub ip: Option<String>,
    /// Truncated User-Agent string. Cap at 256 chars at write time.
    pub user_agent: Option<String>,
    /// True iff `action` succeeded. Failed-login events are still
    /// logged with `success=false` so SIEM can spot brute force.
    pub success: bool,
    /// Free-form short reason on failure ("WRONG_PASSWORD",
    /// "RATE_LIMITED"). Plain strings — no template interpolation.
    pub reason: Option<String>,
    /// Stringly-typed structured metadata. Avoid putting secrets
    /// here; the audit log is meant to be readable by ops.
    pub metadata: HashMap<String, String>,
    /// Entity the event is about (application events), e.g. "Lead".
    #[serde(default)]
    pub entity: Option<String>,
    /// Row id within `entity`.
    #[serde(default)]
    pub entity_id: Option<String>,
}

/// Filters for [`AuditBackend::find`]. Every set field must match.
/// Results are newest first.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    pub tenant_id: Option<String>,
    pub actor_id: Option<String>,
    pub user_id: Option<String>,
    pub entity: Option<String>,
    pub entity_id: Option<String>,
    pub action: Option<String>,
    /// Only events with `created_at` strictly before this (unix seconds).
    pub before: Option<u64>,
    /// Paging cursor: only events older than the event with this id (by
    /// time, then by append order). Unknown ids match nothing.
    pub before_id: Option<String>,
    /// Max rows. Clamped to 1..=1000.
    pub limit: usize,
}

impl AuditQuery {
    pub fn bounded_limit(&self) -> usize {
        self.limit.clamp(1, 1000)
    }

    /// Whether `e` passes every filter. Used by the in-memory backend and
    /// by tests of the SQL backends.
    pub fn matches(&self, e: &AuditEvent) -> bool {
        fn eq(want: &Option<String>, got: &Option<String>) -> bool {
            want.as_ref()
                .is_none_or(|w| got.as_deref() == Some(w.as_str()))
        }
        eq(&self.tenant_id, &e.tenant_id)
            && eq(&self.actor_id, &e.actor_id)
            && eq(&self.user_id, &e.user_id)
            && eq(&self.entity, &e.entity)
            && eq(&self.entity_id, &e.entity_id)
            && self.action.as_ref().is_none_or(|a| e.action.as_str() == a)
            && self.before.is_none_or(|b| e.created_at < b)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    /// Successful sign-in via any path. `metadata.method` carries
    /// the specific path ("password", "magic_code", "magic_link",
    /// "oauth:google", "passkey", "siwe", "phone").
    SignIn,
    SignOut,
    /// Failed sign-in (wrong password, expired magic, bad TOTP).
    SignInFailed,
    /// User row created via any sign-up path.
    SignUp,
    /// Self-service password change (with current password).
    PasswordChange,
    /// Password reset via emailed token.
    PasswordReset,
    EmailChange,
    /// TOTP enrollment finalized (first verify after enroll).
    TotpEnroll,
    /// TOTP disabled by the user.
    TotpDisable,
    /// Backup codes regenerated (invalidates the prior set).
    TotpBackupCodesRegenerate,
    PasskeyRegister,
    PasskeyRevoke,
    ApiKeyCreate,
    ApiKeyRevoke,
    OauthLink,
    OauthUnlink,
    OrgCreate,
    OrgDelete,
    OrgInviteSend,
    OrgInviteAccept,
    OrgMemberRemove,
    OrgRoleChange,
    AccountDelete,
    /// Anonymous (guest) session merged into a real user — fired when
    /// magic-code verify or OAuth callback observes a guest cookie and
    /// transfers ownership of the guest's data to the authenticated
    /// user. `metadata.from_user_id` is the guest id; `metadata.entities`
    /// is a comma-separated list of `<entity>.<field>:<rows>` pairs so
    /// an operator can audit "did the cart actually move?" without SQL.
    AnonymousMerge,
    /// Apps that need a custom event use this with their own string.
    /// Stored verbatim — pylon doesn't validate the content.
    Custom(String),
}

impl AuditAction {
    pub fn as_str(&self) -> &str {
        match self {
            Self::SignIn => "sign_in",
            Self::SignOut => "sign_out",
            Self::SignInFailed => "sign_in_failed",
            Self::SignUp => "sign_up",
            Self::PasswordChange => "password_change",
            Self::PasswordReset => "password_reset",
            Self::EmailChange => "email_change",
            Self::TotpEnroll => "totp_enroll",
            Self::TotpDisable => "totp_disable",
            Self::TotpBackupCodesRegenerate => "totp_backup_codes_regenerate",
            Self::PasskeyRegister => "passkey_register",
            Self::PasskeyRevoke => "passkey_revoke",
            Self::ApiKeyCreate => "api_key_create",
            Self::ApiKeyRevoke => "api_key_revoke",
            Self::OauthLink => "oauth_link",
            Self::OauthUnlink => "oauth_unlink",
            Self::OrgCreate => "org_create",
            Self::OrgDelete => "org_delete",
            Self::OrgInviteSend => "org_invite_send",
            Self::OrgInviteAccept => "org_invite_accept",
            Self::OrgMemberRemove => "org_member_remove",
            Self::OrgRoleChange => "org_role_change",
            Self::AccountDelete => "account_delete",
            Self::AnonymousMerge => "anonymous_merge",
            Self::Custom(s) => s,
        }
    }
}

/// Minimal builder for the per-request fields that the route
/// handler can grab from the RouterContext.
#[derive(Debug, Clone, Default)]
pub struct AuditEventBuilder {
    pub action: Option<AuditAction>,
    pub user_id: Option<String>,
    pub actor_id: Option<String>,
    pub tenant_id: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub success: bool,
    pub reason: Option<String>,
    pub metadata: HashMap<String, String>,
    pub entity: Option<String>,
    pub entity_id: Option<String>,
}

impl AuditEventBuilder {
    pub fn new(action: AuditAction) -> Self {
        Self {
            action: Some(action),
            success: true,
            ..Default::default()
        }
    }
    pub fn user(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }
    pub fn actor(mut self, actor_id: impl Into<String>) -> Self {
        self.actor_id = Some(actor_id.into());
        self
    }
    pub fn tenant(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }
    pub fn ip(mut self, ip: impl Into<String>) -> Self {
        self.ip = Some(ip.into());
        self
    }
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        let s = ua.into();
        // Cap UA at 256 chars — long UAs are usually a fingerprint
        // attempt, not real data. Truncating here saves the SQL
        // backend from oversized rows.
        let truncated: String = s.chars().take(256).collect();
        self.user_agent = Some(truncated);
        self
    }
    pub fn failed(mut self, reason: impl Into<String>) -> Self {
        self.success = false;
        self.reason = Some(reason.into());
        self
    }
    pub fn entity(mut self, entity: impl Into<String>, entity_id: Option<String>) -> Self {
        self.entity = Some(entity.into());
        self.entity_id = entity_id;
        self
    }
    pub fn meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
    pub fn build(self) -> AuditEvent {
        let action = self.action.unwrap_or(AuditAction::Custom("unknown".into()));
        AuditEvent {
            id: format!("evt_{}", random_token(20)),
            created_at: now_secs(),
            action,
            user_id: self.user_id,
            actor_id: self.actor_id,
            tenant_id: self.tenant_id,
            ip: self.ip,
            user_agent: self.user_agent,
            success: self.success,
            reason: self.reason,
            metadata: self.metadata,
            entity: self.entity,
            entity_id: self.entity_id,
        }
    }
}

pub trait AuditBackend: Send + Sync {
    /// Write one event. Errors when it could not be stored.
    fn try_append(&self, event: &AuditEvent) -> Result<(), String>;
    /// Write one event, logging a failure instead of returning it. Auth
    /// flows use this so a broken audit sink never blocks sign-in.
    fn append(&self, event: &AuditEvent) {
        if let Err(e) = self.try_append(event) {
            tracing::warn!(
                "[audit] failed to store {} event: {e}",
                event.action.as_str()
            );
        }
    }
    /// Events matching every filter in `query`, newest first.
    fn find(&self, query: &AuditQuery) -> Result<Vec<AuditEvent>, String>;
    /// Tenant-scoped query. Returns at most `limit` events newest-first.
    /// Backends MUST respect `tenant_id` to prevent cross-tenant leak.
    fn find_for_tenant(&self, tenant_id: &str, limit: usize) -> Vec<AuditEvent>;
    /// User-scoped query. Returns events where the user is the
    /// subject OR the actor.
    fn find_for_user(&self, user_id: &str, limit: usize) -> Vec<AuditEvent>;
}

pub struct InMemoryAuditBackend {
    events: Mutex<Vec<AuditEvent>>,
}

impl Default for InMemoryAuditBackend {
    fn default() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl AuditBackend for InMemoryAuditBackend {
    fn try_append(&self, event: &AuditEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
    fn find(&self, query: &AuditQuery) -> Result<Vec<AuditEvent>, String> {
        let g = self.events.lock().unwrap();
        // (created_at, append position) orders events; a cursor keeps only
        // events strictly older than its event.
        let cursor = match &query.before_id {
            None => None,
            Some(id) => match g.iter().position(|e| &e.id == id) {
                Some(pos) => Some((g[pos].created_at, pos)),
                None => return Ok(Vec::new()),
            },
        };
        let mut out: Vec<(u64, usize, AuditEvent)> = g
            .iter()
            .enumerate()
            .filter(|(_, e)| query.matches(e))
            .filter(|(pos, e)| cursor.is_none_or(|c| (e.created_at, *pos) < c))
            .map(|(pos, e)| (e.created_at, pos, e.clone()))
            .collect();
        out.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
        out.truncate(query.bounded_limit());
        Ok(out.into_iter().map(|(_, _, e)| e).collect())
    }
    fn find_for_tenant(&self, tenant_id: &str, limit: usize) -> Vec<AuditEvent> {
        let g = self.events.lock().unwrap();
        let mut out: Vec<AuditEvent> = g
            .iter()
            .filter(|e| e.tenant_id.as_deref() == Some(tenant_id))
            .cloned()
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out.truncate(limit);
        out
    }
    fn find_for_user(&self, user_id: &str, limit: usize) -> Vec<AuditEvent> {
        let g = self.events.lock().unwrap();
        let mut out: Vec<AuditEvent> = g
            .iter()
            .filter(|e| {
                e.user_id.as_deref() == Some(user_id) || e.actor_id.as_deref() == Some(user_id)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out.truncate(limit);
        out
    }
}

pub struct AuditStore {
    backend: Box<dyn AuditBackend>,
}

impl Default for AuditStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditStore {
    pub fn new() -> Self {
        Self::with_backend(Box::new(InMemoryAuditBackend::default()))
    }
    pub fn with_backend(backend: Box<dyn AuditBackend>) -> Self {
        Self { backend }
    }

    /// Convenience: build + append in one call. Most call sites do
    /// `store.log(AuditEventBuilder::new(...).user(...).build())`.
    pub fn log(&self, event: AuditEvent) {
        self.backend.append(&event);
    }

    /// Write one event and report a storage failure. Application audit
    /// events use this: a record that silently fails to save is worse
    /// than an error.
    pub fn try_log(&self, event: &AuditEvent) -> Result<(), String> {
        self.backend.try_append(event)
    }

    pub fn find(&self, query: &AuditQuery) -> Result<Vec<AuditEvent>, String> {
        self.backend.find(query)
    }

    pub fn find_for_tenant(&self, tenant_id: &str, limit: usize) -> Vec<AuditEvent> {
        self.backend.find_for_tenant(tenant_id, limit)
    }
    pub fn find_for_user(&self, user_id: &str, limit: usize) -> Vec<AuditEvent> {
        self.backend.find_for_user(user_id, limit)
    }
}

fn random_token(n_bytes: usize) -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; n_bytes];
    rand::thread_rng().fill_bytes(&mut bytes);
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD.encode(bytes)
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_default_success_true() {
        let e = AuditEventBuilder::new(AuditAction::SignIn).build();
        assert!(e.success);
        assert!(e.reason.is_none());
    }

    #[test]
    fn builder_failed_flips_success_and_records_reason() {
        let e = AuditEventBuilder::new(AuditAction::SignInFailed)
            .failed("WRONG_PASSWORD")
            .build();
        assert!(!e.success);
        assert_eq!(e.reason.as_deref(), Some("WRONG_PASSWORD"));
    }

    #[test]
    fn user_agent_truncated_to_256_chars() {
        let huge_ua = "X".repeat(2000);
        let e = AuditEventBuilder::new(AuditAction::SignIn)
            .user_agent(huge_ua)
            .build();
        assert_eq!(e.user_agent.as_ref().unwrap().chars().count(), 256);
    }

    #[test]
    fn tenant_query_isolates_cross_tenant() {
        // Critical isolation check: events tagged with tenant=A
        // must NEVER leak into a tenant=B query.
        let s = AuditStore::new();
        s.log(
            AuditEventBuilder::new(AuditAction::SignIn)
                .tenant("tenant_a")
                .user("u1")
                .build(),
        );
        s.log(
            AuditEventBuilder::new(AuditAction::SignIn)
                .tenant("tenant_b")
                .user("u2")
                .build(),
        );
        s.log(
            AuditEventBuilder::new(AuditAction::SignIn)
                // No tenant — should not leak into either query.
                .user("u3")
                .build(),
        );
        let a = s.find_for_tenant("tenant_a", 100);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].user_id.as_deref(), Some("u1"));
        let b = s.find_for_tenant("tenant_b", 100);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].user_id.as_deref(), Some("u2"));
    }

    #[test]
    fn user_query_returns_subject_and_actor_events() {
        // An admin disabling a user's account: actor=admin, user=alice.
        // Both queries should surface it (alice sees what happened to
        // her; admin sees what they did).
        let s = AuditStore::new();
        s.log(
            AuditEventBuilder::new(AuditAction::AccountDelete)
                .user("alice")
                .actor("admin")
                .build(),
        );
        assert_eq!(s.find_for_user("alice", 100).len(), 1);
        assert_eq!(s.find_for_user("admin", 100).len(), 1);
        assert_eq!(s.find_for_user("bob", 100).len(), 0);
    }

    #[test]
    fn newest_first_ordering() {
        let s = AuditStore::new();
        // Inject events with explicit timestamps to defeat clock noise.
        s.backend.append(&AuditEvent {
            id: "evt_a".into(),
            created_at: 100,
            action: AuditAction::SignIn,
            user_id: Some("u".into()),
            actor_id: None,
            tenant_id: Some("t".into()),
            ip: None,
            user_agent: None,
            success: true,
            reason: None,
            metadata: HashMap::new(),
            entity: None,
            entity_id: None,
        });
        s.backend.append(&AuditEvent {
            id: "evt_b".into(),
            created_at: 200,
            action: AuditAction::SignOut,
            user_id: Some("u".into()),
            actor_id: None,
            tenant_id: Some("t".into()),
            ip: None,
            user_agent: None,
            success: true,
            reason: None,
            metadata: HashMap::new(),
            entity: None,
            entity_id: None,
        });
        let out = s.find_for_tenant("t", 10);
        assert_eq!(out[0].id, "evt_b"); // newest first
        assert_eq!(out[1].id, "evt_a");
    }

    #[test]
    fn limit_caps_results() {
        let s = AuditStore::new();
        for i in 0..50 {
            s.log(
                AuditEventBuilder::new(AuditAction::SignIn)
                    .tenant("t")
                    .user(format!("u_{i}"))
                    .build(),
            );
        }
        assert_eq!(s.find_for_tenant("t", 10).len(), 10);
    }

    #[test]
    fn metadata_preserves_string_only_values() {
        // Defends against a future caller passing JSON values that
        // could contain nested PII or tokens. Stringly-typed by design.
        let e = AuditEventBuilder::new(AuditAction::SignIn)
            .meta("method", "oauth:google")
            .meta("device", "iPhone")
            .build();
        assert_eq!(
            e.metadata.get("method").map(|s| s.as_str()),
            Some("oauth:google")
        );
        assert_eq!(e.metadata.len(), 2);
    }

    #[test]
    fn custom_action_serializes_verbatim() {
        let e = AuditEventBuilder::new(AuditAction::Custom(
            "pylon.cloud.fly_machine_provision".into(),
        ))
        .build();
        assert_eq!(e.action.as_str(), "pylon.cloud.fly_machine_provision");
    }

    #[test]
    fn no_tenant_event_invisible_to_tenant_query() {
        // System events without tenant context must never accidentally
        // surface in a tenant-scoped query.
        let s = AuditStore::new();
        s.log(AuditEventBuilder::new(AuditAction::Custom("system.tick".into())).build());
        assert_eq!(s.find_for_tenant("tenant_a", 100).len(), 0);
    }

    #[test]
    fn find_applies_every_filter() {
        let s = AuditStore::new();
        s.log(
            AuditEventBuilder::new(AuditAction::Custom("app.lead.export".into()))
                .actor("rep1")
                .tenant("dealer_a")
                .entity("Lead", Some("l1".into()))
                .build(),
        );
        s.log(
            AuditEventBuilder::new(AuditAction::Custom("app.lead.view".into()))
                .actor("rep2")
                .tenant("dealer_a")
                .entity("Lead", Some("l2".into()))
                .build(),
        );
        s.log(
            AuditEventBuilder::new(AuditAction::Custom("app.lead.export".into()))
                .actor("rep3")
                .tenant("dealer_b")
                .entity("Lead", Some("l1".into()))
                .build(),
        );
        let q = |f: fn(&mut AuditQuery)| {
            let mut q = AuditQuery {
                tenant_id: Some("dealer_a".into()),
                limit: 50,
                ..Default::default()
            };
            f(&mut q);
            s.find(&q).unwrap()
        };
        assert_eq!(q(|_| {}).len(), 2);
        assert_eq!(q(|q| q.entity_id = Some("l1".into())).len(), 1);
        assert_eq!(q(|q| q.action = Some("app.lead.view".into())).len(), 1);
        assert_eq!(q(|q| q.actor_id = Some("rep3".into())).len(), 0);
    }
}
