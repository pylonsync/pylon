//! Organizations + memberships + invites — multi-tenant team management
//! backed by manifest-declared entities.
//!
//! The framework's `/api/auth/orgs/*` surface reads + writes through
//! the app's DataStore using entity names from `ManifestAuthOrgConfig`.
//! Apps declare three entities in their schema (`Org`, `OrgMember`,
//! `OrgInvite` by default — names configurable) and add whatever extra
//! fields they need (logo, industry, billingEmail, etc.) without
//! touching the framework.
//!
//! ## Required entity shapes
//!
//! **Org**
//!   - `id: string` (auto)
//!   - `name: string`
//!   - `createdBy: string` (User id)
//!   - `createdAt: string` (ISO 8601)
//!
//! **OrgMember**
//!   - `id: string` (auto)
//!   - `orgId: string` (relation to Org)
//!   - `userId: string` (relation to User)
//!   - `role: string` (owner | admin | member)
//!   - `joinedAt: string` (ISO 8601)
//!
//! **OrgInvite**
//!   - `id: string` — `inv_<random>`
//!   - `orgId: string`
//!   - `email: string` (lowercased)
//!   - `role: string`
//!   - `invitedBy: string` (User id)
//!   - `tokenHash: string` (Argon2)
//!   - `tokenPrefix: string` (first 8 chars of plaintext, for display)
//!   - `createdAt: string`
//!   - `expiresAt: string`
//!   - `acceptedAt: string?` (null until consumed)
//!
//! Apps free to add any other fields. Reads return the full row;
//! writes set only the framework-managed fields.

use std::sync::Arc;

use pylon_http::DataStore;
use pylon_kernel::ManifestAuthOrgConfig;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types — same surface external callers used pre-v0.3.74
// ---------------------------------------------------------------------------

/// Role within an organization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrgRole {
    /// Can do everything, including deleting the org and reassigning
    /// ownership. Multiple owners allowed (pass an existing owner's
    /// successor before they leave).
    Owner,
    /// Manage members + invites + most settings, but cannot delete
    /// the org or transfer ownership.
    Admin,
    /// Default role for invited members.
    Member,
}

impl OrgRole {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }
    pub fn can_manage_members(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
    pub fn can_delete_org(&self) -> bool {
        matches!(self, Self::Owner)
    }
    pub fn can_transfer_ownership(&self) -> bool {
        matches!(self, Self::Owner)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Org {
    pub id: String,
    pub name: String,
    pub created_by: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    pub org_id: String,
    pub user_id: String,
    pub role: OrgRole,
    pub joined_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub id: String,
    pub org_id: String,
    pub email: String,
    pub role: OrgRole,
    pub invited_by: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub accepted_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct InviteWithToken {
    pub invite: Invite,
    /// Plaintext token — show in `accept_url`, never persist. Lost
    /// after this method returns.
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptError {
    /// Token doesn't match any stored invite.
    NotFound,
    /// Invite is past `expires_at`.
    Expired,
    /// Invite was already redeemed.
    AlreadyAccepted,
    /// The accepting user's email doesn't match the invite's addressee.
    EmailMismatch,
    /// User is already a member of this org via a different path.
    AlreadyMember,
}

impl std::fmt::Display for AcceptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFound => "invite not found",
            Self::Expired => "invite expired",
            Self::AlreadyAccepted => "invite already accepted",
            Self::EmailMismatch => "invite email doesn't match this account",
            Self::AlreadyMember => "user is already a member of this org",
        })
    }
}

// ---------------------------------------------------------------------------
// OrgStore — DataStore-backed org/member/invite operations
// ---------------------------------------------------------------------------

/// All org / member / invite operations the framework's
/// `/api/auth/orgs/*` route handlers need, routed through the app's
/// DataStore.
///
/// Apps customize the underlying entities by adding fields in their
/// schema; the framework reads + writes only the fields documented in
/// the module header.
pub struct OrgStore {
    store: Arc<dyn DataStore>,
    cfg: ManifestAuthOrgConfig,
}

impl OrgStore {
    pub fn new(store: Arc<dyn DataStore>, cfg: ManifestAuthOrgConfig) -> Self {
        Self { store, cfg }
    }

    fn is_disabled(&self) -> bool {
        self.cfg.disabled
    }

    /// Whether the org surface is enabled in this app's manifest.
    /// Routes check this and 501 when disabled.
    pub fn enabled(&self) -> bool {
        !self.is_disabled()
    }

    // ----- Org -----

    /// Create an org. Creator becomes Owner via an OrgMember row.
    pub fn create(&self, name: &str, creator_id: &str) -> Option<Org> {
        if self.is_disabled() {
            return None;
        }
        let now = now_secs();
        let now_iso = now_iso();
        let payload = serde_json::json!({
            "name": name,
            "createdBy": creator_id,
            "createdAt": now_iso,
        });
        let id = match self.store.insert(&self.cfg.entity, &payload) {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("[org] create failed: {} {}", e.code, e.message);
                return None;
            }
        };
        // Seed Owner membership.
        let member_payload = serde_json::json!({
            "orgId": id,
            "userId": creator_id,
            "role": OrgRole::Owner.as_str(),
            "joinedAt": now_iso,
        });
        if let Err(e) = self.store.insert(&self.cfg.member_entity, &member_payload) {
            // Roll back the org row so we don't leave a name-claimed
            // org with no members.
            let _ = self.store.delete(&self.cfg.entity, &id);
            tracing::warn!("[org] member seed failed: {} {}", e.code, e.message);
            return None;
        }
        Some(Org {
            id,
            name: name.to_string(),
            created_by: creator_id.to_string(),
            created_at: now,
        })
    }

    pub fn get(&self, org_id: &str) -> Option<Org> {
        if self.is_disabled() {
            return None;
        }
        let row = self
            .store
            .get_by_id(&self.cfg.entity, org_id)
            .ok()
            .flatten()?;
        Some(row_to_org(&row))
    }

    /// Rename an org. Returns true when the row was updated. Org metadata is
    /// owned by the framework (the Org entity is policy-locked to no client
    /// writes), so renames go through here, not a sync mutation.
    pub fn rename(&self, org_id: &str, name: &str) -> bool {
        if self.is_disabled() {
            return false;
        }
        self.store
            .update(&self.cfg.entity, org_id, &serde_json::json!({ "name": name }))
            .unwrap_or(false)
    }

    /// Delete an org (and its memberships + pending invites).
    pub fn delete(&self, org_id: &str) -> bool {
        if self.is_disabled() {
            return false;
        }
        // Wipe child rows first so we don't strand them.
        for m in self.list_members(org_id) {
            let _ = self.delete_member_row(&m);
        }
        for inv in self.list_invites(org_id) {
            let _ = self.store.delete(&self.cfg.invite_entity, &inv.id);
        }
        self.store.delete(&self.cfg.entity, org_id).unwrap_or(false)
    }

    pub fn list_for_user(&self, user_id: &str) -> Vec<(Org, OrgRole)> {
        if self.is_disabled() {
            return Vec::new();
        }
        let memberships = match self.store.query_filtered(
            &self.cfg.member_entity,
            &serde_json::json!({ "userId": user_id }),
        ) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::with_capacity(memberships.len());
        for m in memberships {
            let org_id = match m.get("orgId").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let role = m
                .get("role")
                .and_then(|v| v.as_str())
                .and_then(OrgRole::from_str)
                .unwrap_or(OrgRole::Member);
            if let Some(org) = self.get(&org_id) {
                out.push((org, role));
            }
        }
        out
    }

    // ----- Members -----

    pub fn list_members(&self, org_id: &str) -> Vec<Membership> {
        if self.is_disabled() {
            return Vec::new();
        }
        self.store
            .query_filtered(
                &self.cfg.member_entity,
                &serde_json::json!({ "orgId": org_id }),
            )
            .unwrap_or_default()
            .iter()
            .map(row_to_membership)
            .collect()
    }

    pub fn role_of(&self, org_id: &str, user_id: &str) -> Option<OrgRole> {
        if self.is_disabled() {
            return None;
        }
        self.find_member_row(org_id, user_id)
            .and_then(|m| m.get("role").and_then(|v| v.as_str()).map(String::from))
            .and_then(|s| OrgRole::from_str(&s))
    }

    /// Update an existing member's role. Returns true if a row matched.
    pub fn set_role(&self, org_id: &str, user_id: &str, role: OrgRole) -> bool {
        if self.is_disabled() {
            return false;
        }
        let Some(row) = self.find_member_row(org_id, user_id) else {
            return false;
        };
        let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
            return false;
        };
        self.store
            .update(
                &self.cfg.member_entity,
                id,
                &serde_json::json!({ "role": role.as_str() }),
            )
            .unwrap_or(false)
    }

    /// Idempotent membership creation. If the user is already a member,
    /// preserve the existing role.
    pub fn add_member(&self, org_id: &str, user_id: &str, role: OrgRole) -> OrgRole {
        if self.is_disabled() {
            return role;
        }
        if let Some(existing_role) = self.role_of(org_id, user_id) {
            return existing_role;
        }
        let _ = self.store.insert(
            &self.cfg.member_entity,
            &serde_json::json!({
                "orgId": org_id,
                "userId": user_id,
                "role": role.as_str(),
                "joinedAt": now_iso(),
            }),
        );
        role
    }

    pub fn remove_member(&self, org_id: &str, user_id: &str) -> bool {
        if self.is_disabled() {
            return false;
        }
        let Some(row) = self.find_member_row(org_id, user_id) else {
            return false;
        };
        let Some(id) = row.get("id").and_then(|v| v.as_str()).map(String::from) else {
            return false;
        };
        self.store
            .delete(&self.cfg.member_entity, &id)
            .unwrap_or(false)
    }

    fn find_member_row(&self, org_id: &str, user_id: &str) -> Option<serde_json::Value> {
        self.store
            .query_filtered(
                &self.cfg.member_entity,
                &serde_json::json!({ "orgId": org_id, "userId": user_id }),
            )
            .ok()
            .and_then(|rows| rows.into_iter().next())
    }

    fn delete_member_row(&self, m: &Membership) -> Option<()> {
        let row = self.find_member_row(&m.org_id, &m.user_id)?;
        let id = row.get("id").and_then(|v| v.as_str())?;
        let _ = self.store.delete(&self.cfg.member_entity, id);
        Some(())
    }

    // ----- Invites -----
    //
    // create_invite + accept_invite need argon2-backed password
    // hashing (crate::password). argon2 doesn't cross-compile to
    // wasm32, so both methods are gated for the Workers target.
    // Workers-side org code that needs invitation flows would
    // need a WebCrypto-based replacement; the rest of OrgStore
    // (members, roles, list_invites, revoke_invite) stays
    // available on every target.

    #[cfg(not(target_arch = "wasm32"))]
    pub fn create_invite(
        &self,
        org_id: &str,
        email: &str,
        role: OrgRole,
        invited_by: &str,
    ) -> Option<InviteWithToken> {
        if self.is_disabled() {
            return None;
        }
        let token = random_token(24);
        let token_hash = crate::password::hash_password(&token);
        let token_prefix: String = token.chars().take(8).collect();
        let created_at = now_secs();
        let expires_at = created_at + 7 * 24 * 60 * 60;
        let payload = serde_json::json!({
            "orgId": org_id,
            "email": email.to_lowercase(),
            "role": role.as_str(),
            "invitedBy": invited_by,
            "tokenHash": token_hash.clone(),
            "tokenPrefix": token_prefix.clone(),
            "createdAt": iso(created_at),
            "expiresAt": iso(expires_at),
        });
        let id = self.store.insert(&self.cfg.invite_entity, &payload).ok()?;
        Some(InviteWithToken {
            invite: Invite {
                id,
                org_id: org_id.to_string(),
                email: email.to_lowercase(),
                role,
                invited_by: invited_by.to_string(),
                token_hash,
                token_prefix,
                created_at,
                expires_at,
                accepted_at: None,
            },
            token,
        })
    }

    pub fn list_invites(&self, org_id: &str) -> Vec<Invite> {
        if self.is_disabled() {
            return Vec::new();
        }
        self.store
            .query_filtered(
                &self.cfg.invite_entity,
                &serde_json::json!({ "orgId": org_id }),
            )
            .unwrap_or_default()
            .iter()
            .map(row_to_invite)
            .collect()
    }

    pub fn revoke_invite(&self, invite_id: &str) -> bool {
        if self.is_disabled() {
            return false;
        }
        self.store
            .delete(&self.cfg.invite_entity, invite_id)
            .unwrap_or(false)
    }

    /// Accept an invite. Verifies the token (Argon2 hash compare),
    /// checks expiry + accepted-at, ensures the accepting user's email
    /// matches the invite, CAS-stamps `acceptedAt`, then inserts the
    /// membership row.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn accept_invite(
        &self,
        token: &str,
        accepting_user_id: &str,
        accepting_email: &str,
    ) -> Result<Membership, AcceptError> {
        if self.is_disabled() {
            return Err(AcceptError::NotFound);
        }
        // Narrow by token prefix (cheap query index) then Argon2-verify
        // candidates. Argon2 is non-deterministic so we can't look up
        // by hash directly.
        let prefix: String = token.chars().take(8).collect();
        let candidates = self
            .store
            .query_filtered(
                &self.cfg.invite_entity,
                &serde_json::json!({ "tokenPrefix": prefix }),
            )
            .map_err(|_| AcceptError::NotFound)?;
        let mut invite_row: Option<serde_json::Value> = None;
        for row in candidates {
            let Some(stored_hash) = row.get("tokenHash").and_then(|v| v.as_str()) else {
                continue;
            };
            if crate::password::verify_password(token, stored_hash) {
                invite_row = Some(row);
                break;
            }
        }
        let row = invite_row.ok_or(AcceptError::NotFound)?;
        let invite = row_to_invite(&row);
        if invite.accepted_at.is_some() {
            return Err(AcceptError::AlreadyAccepted);
        }
        if invite.expires_at <= now_secs() {
            return Err(AcceptError::Expired);
        }
        if invite.email != accepting_email.to_lowercase() {
            return Err(AcceptError::EmailMismatch);
        }
        if self.role_of(&invite.org_id, accepting_user_id).is_some() {
            return Err(AcceptError::AlreadyMember);
        }
        // CAS-stamp acceptedAt BEFORE creating the membership. The
        // `update` returns true on a successful row write — for a real
        // CAS we'd want a "compare and set acceptedAt only if null"
        // primitive, but the DataStore trait doesn't expose that today.
        // Instead we update + re-read to verify the value we observed.
        // Two parallel verifies that both pass the accepted_at=None
        // check race here; the loser sees the winner's stamp on
        // re-read and bails. (Race window shrinks further once we add
        // a real CAS update on the DataStore trait.)
        let now = now_secs();
        let now_iso = iso(now);
        let updated = self
            .store
            .update(
                &self.cfg.invite_entity,
                &invite.id,
                &serde_json::json!({ "acceptedAt": now_iso }),
            )
            .unwrap_or(false);
        if !updated {
            return Err(AcceptError::NotFound);
        }
        let after = self
            .store
            .get_by_id(&self.cfg.invite_entity, &invite.id)
            .ok()
            .flatten()
            .ok_or(AcceptError::NotFound)?;
        let after_accepted = after
            .get("acceptedAt")
            .and_then(|v| v.as_str())
            .map(String::from);
        if after_accepted.as_deref() != Some(now_iso.as_str()) {
            // A parallel verify won the CAS.
            return Err(AcceptError::AlreadyAccepted);
        }
        let membership_payload = serde_json::json!({
            "orgId": invite.org_id,
            "userId": accepting_user_id,
            "role": invite.role.as_str(),
            "joinedAt": now_iso,
        });
        self.store
            .insert(&self.cfg.member_entity, &membership_payload)
            .map_err(|_| AcceptError::NotFound)?;
        Ok(Membership {
            org_id: invite.org_id,
            user_id: accepting_user_id.to_string(),
            role: invite.role,
            joined_at: now,
        })
    }
}

// ---------------------------------------------------------------------------
// Row → struct helpers
// ---------------------------------------------------------------------------

fn row_to_org(row: &serde_json::Value) -> Org {
    Org {
        id: row.get("id").and_then(|v| v.as_str()).unwrap_or("").into(),
        name: row
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        created_by: row
            .get("createdBy")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        created_at: parse_unix_seconds(row.get("createdAt")),
    }
}

fn row_to_membership(row: &serde_json::Value) -> Membership {
    Membership {
        org_id: row
            .get("orgId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        user_id: row
            .get("userId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        role: row
            .get("role")
            .and_then(|v| v.as_str())
            .and_then(OrgRole::from_str)
            .unwrap_or(OrgRole::Member),
        joined_at: parse_unix_seconds(row.get("joinedAt")),
    }
}

fn row_to_invite(row: &serde_json::Value) -> Invite {
    Invite {
        id: row.get("id").and_then(|v| v.as_str()).unwrap_or("").into(),
        org_id: row
            .get("orgId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        email: row
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        role: row
            .get("role")
            .and_then(|v| v.as_str())
            .and_then(OrgRole::from_str)
            .unwrap_or(OrgRole::Member),
        invited_by: row
            .get("invitedBy")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        token_hash: row
            .get("tokenHash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        token_prefix: row
            .get("tokenPrefix")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        created_at: parse_unix_seconds(row.get("createdAt")),
        expires_at: parse_unix_seconds(row.get("expiresAt")),
        accepted_at: row
            .get("acceptedAt")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(parse_unix_seconds_str),
    }
}

fn parse_unix_seconds(v: Option<&serde_json::Value>) -> u64 {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(serde_json::Value::String(s)) => parse_unix_seconds_str(s),
        _ => 0,
    }
}

fn parse_unix_seconds_str(s: &str) -> u64 {
    // Accept either pure unix-seconds strings ("1735000000") or ISO 8601.
    if let Ok(n) = s.parse::<u64>() {
        return n;
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp().max(0) as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Time + random utilities
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_iso() -> String {
    iso(now_secs())
}

fn iso(unix_seconds: u64) -> String {
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(unix_seconds as i64, 0)
        .unwrap_or_else(chrono::Utc::now);
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn random_token(n_bytes: usize) -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = vec![0u8; n_bytes];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Populate `auth.roles` with the caller's role in their active org.
///
/// `SessionStore::resolve` → `to_auth_context` only copies `user_id` +
/// `tenant_id`; it leaves `roles` empty for org members because the session
/// store has no view of the entity-backed membership table. That left
/// `auth.roles` permanently empty for every signed-in user, so any gate on it
/// — the client `Protect` / `InviteMembers` components, server-side policies
/// branching on "owner"/"admin"/"member" — was dead. The visible symptom: the
/// invite UI was hidden even from the org's own owner.
///
/// This runs once per request, after session resolution and admin promotion,
/// to fill that gap from the same `OrgMember` table the server already trusts
/// for enforcement. Conservative + idempotent:
///   - never overwrites a non-empty `roles` list (admin contexts carry
///     `["admin"]`; JWT sessions carry their claim roles),
///   - skips admin and API-key contexts (a scoped `pk.*` token must not pick
///     up org roles it wasn't minted with),
///   - only fills when there's an active tenant AND a membership row in it.
pub fn enrich_active_org_role(orgs: &OrgStore, auth: &mut crate::AuthContext) {
    if !auth.roles.is_empty() || auth.is_admin || auth.is_api_key_auth() {
        return;
    }
    let (Some(tenant), Some(uid)) = (auth.tenant_id.clone(), auth.user_id.clone()) else {
        return;
    };
    if let Some(role) = orgs.role_of(&tenant, &uid) {
        auth.roles = vec![role.as_str().to_string()];
    }
}

// ---------------------------------------------------------------------------
// Tests — in-memory DataStore exercising the entity-backed OrgStore
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_http::{DataError, DataStore};
    use pylon_kernel::{AppManifest, MANIFEST_VERSION};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Tiny in-memory DataStore for org-flow tests. Implements just
    /// enough of the DataStore trait for OrgStore to round-trip.
    struct InMemStore {
        manifest: AppManifest,
        // entity -> (id -> row)
        rows: Mutex<HashMap<String, HashMap<String, serde_json::Value>>>,
        next_id: Mutex<u64>,
    }

    impl InMemStore {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                manifest: AppManifest {
                    manifest_version: MANIFEST_VERSION,
                    name: "test".into(),
                    version: "0".into(),
                    ..Default::default()
                },
                rows: Mutex::new(HashMap::new()),
                next_id: Mutex::new(0),
            })
        }
    }

    impl DataStore for InMemStore {
        fn manifest(&self) -> &AppManifest {
            &self.manifest
        }
        fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError> {
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            let id = format!("{}_{}", entity.to_lowercase(), *next);
            let mut row = data.clone();
            if let Some(o) = row.as_object_mut() {
                o.insert("id".into(), serde_json::Value::String(id.clone()));
            }
            self.rows
                .lock()
                .unwrap()
                .entry(entity.to_string())
                .or_default()
                .insert(id.clone(), row);
            Ok(id)
        }
        fn get_by_id(
            &self,
            entity: &str,
            id: &str,
        ) -> Result<Option<serde_json::Value>, DataError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .get(entity)
                .and_then(|m| m.get(id).cloned()))
        }
        fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .get(entity)
                .map(|m| m.values().cloned().collect())
                .unwrap_or_default())
        }
        fn list_after(
            &self,
            entity: &str,
            _after: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<serde_json::Value>, DataError> {
            self.list(entity)
        }
        fn update(
            &self,
            entity: &str,
            id: &str,
            data: &serde_json::Value,
        ) -> Result<bool, DataError> {
            let mut g = self.rows.lock().unwrap();
            let Some(m) = g.get_mut(entity) else {
                return Ok(false);
            };
            let Some(existing) = m.get_mut(id) else {
                return Ok(false);
            };
            if let (Some(ex_obj), Some(new_obj)) = (existing.as_object_mut(), data.as_object()) {
                for (k, v) in new_obj {
                    ex_obj.insert(k.clone(), v.clone());
                }
            }
            Ok(true)
        }
        fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .get_mut(entity)
                .and_then(|m| m.remove(id))
                .is_some())
        }
        fn lookup(
            &self,
            _entity: &str,
            _field: &str,
            _value: &str,
        ) -> Result<Option<serde_json::Value>, DataError> {
            Ok(None)
        }
        fn link(
            &self,
            _entity: &str,
            _id: &str,
            _relation: &str,
            _target_id: &str,
        ) -> Result<bool, DataError> {
            Ok(true)
        }
        fn unlink(&self, _entity: &str, _id: &str, _relation: &str) -> Result<bool, DataError> {
            Ok(true)
        }
        fn query_filtered(
            &self,
            entity: &str,
            filter: &serde_json::Value,
        ) -> Result<Vec<serde_json::Value>, DataError> {
            let obj = filter.as_object();
            Ok(self
                .rows
                .lock()
                .unwrap()
                .get(entity)
                .map(|m| {
                    m.values()
                        .filter(|row| match (obj, row.as_object()) {
                            (Some(f), Some(r)) => f.iter().all(|(k, v)| r.get(k) == Some(v)),
                            _ => true,
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default())
        }
        fn query_graph(&self, _query: &serde_json::Value) -> Result<serde_json::Value, DataError> {
            Ok(serde_json::json!({}))
        }
        fn transact(
            &self,
            _ops: &[serde_json::Value],
        ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
            Ok((true, vec![]))
        }
    }

    fn store() -> OrgStore {
        OrgStore::new(InMemStore::new(), ManifestAuthOrgConfig::default())
    }

    #[test]
    fn create_org_seeds_owner_membership() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();
        assert_eq!(org.name, "Acme");
        assert_eq!(org.created_by, "u-alice");
        assert_eq!(s.role_of(&org.id, "u-alice"), Some(OrgRole::Owner));
    }

    #[test]
    fn list_for_user_returns_owned_org() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();
        let list = s.list_for_user("u-alice");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0.id, org.id);
        assert_eq!(list[0].1, OrgRole::Owner);
    }

    #[test]
    fn rename_updates_the_org_name() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();
        assert!(s.rename(&org.id, "Acme Inc"));
        assert_eq!(s.get(&org.id).unwrap().name, "Acme Inc");
        // Unknown org → no row updated.
        assert!(!s.rename("org-nope", "Whatever"));
    }

    #[test]
    fn add_member_is_idempotent() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();
        let role = s.add_member(&org.id, "u-bob", OrgRole::Member);
        assert_eq!(role, OrgRole::Member);
        // Add again with a "lower" role — the existing role is preserved.
        s.set_role(&org.id, "u-bob", OrgRole::Admin);
        let role_again = s.add_member(&org.id, "u-bob", OrgRole::Member);
        assert_eq!(role_again, OrgRole::Admin);
    }

    #[test]
    fn set_role_updates_membership() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();
        s.add_member(&org.id, "u-bob", OrgRole::Member);
        assert!(s.set_role(&org.id, "u-bob", OrgRole::Admin));
        assert_eq!(s.role_of(&org.id, "u-bob"), Some(OrgRole::Admin));
    }

    // Regression: `to_auth_context` leaves `roles` empty, so without this
    // enrichment every org member (the owner included) failed role gates like
    // `InviteMembers`' `roles.includes("owner"|"admin")` check — the invite UI
    // was hidden from the very person who just created the org.
    #[test]
    fn enrich_fills_active_org_role_for_owner_and_member() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();
        s.add_member(&org.id, "u-bob", OrgRole::Member);

        let mut owner = crate::AuthContext::user("u-alice".into()).with_tenant(org.id.clone());
        enrich_active_org_role(&s, &mut owner);
        assert_eq!(owner.roles, vec!["owner".to_string()]);

        let mut member = crate::AuthContext::user("u-bob".into()).with_tenant(org.id.clone());
        enrich_active_org_role(&s, &mut member);
        assert_eq!(member.roles, vec!["member".to_string()]);
    }

    #[test]
    fn enrich_is_conservative() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();

        // No active tenant → nothing to resolve, roles stay empty.
        let mut no_tenant = crate::AuthContext::user("u-alice".into());
        enrich_active_org_role(&s, &mut no_tenant);
        assert!(no_tenant.roles.is_empty());

        // Active tenant but the caller isn't a member → no role granted.
        let mut stranger = crate::AuthContext::user("u-stranger".into()).with_tenant(org.id.clone());
        enrich_active_org_role(&s, &mut stranger);
        assert!(stranger.roles.is_empty());

        // Admin contexts already carry ["admin"] and must not be clobbered
        // with an org role (or skipped into emptiness).
        let mut admin = crate::AuthContext::admin().with_tenant(org.id.clone());
        enrich_active_org_role(&s, &mut admin);
        assert_eq!(admin.roles, vec!["admin".to_string()]);

        // A scoped API-key context must not pick up org roles it wasn't
        // minted with, even for a real member user.
        let mut api_key =
            crate::AuthContext::from_api_key("u-alice".into(), "key_1".into(), None)
                .with_tenant(org.id.clone());
        enrich_active_org_role(&s, &mut api_key);
        assert!(api_key.roles.is_empty());
    }

    #[test]
    fn invite_round_trip_accept() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();
        let invited = s
            .create_invite(&org.id, "Bob@Example.com", OrgRole::Admin, "u-alice")
            .unwrap();
        // Email is lowercased on store.
        assert_eq!(invited.invite.email, "bob@example.com");
        // Accept with the matching email creates the membership.
        let membership = s
            .accept_invite(&invited.token, "u-bob", "bob@example.com")
            .unwrap();
        assert_eq!(membership.role, OrgRole::Admin);
        assert_eq!(s.role_of(&org.id, "u-bob"), Some(OrgRole::Admin));
    }

    #[test]
    fn invite_accept_rejects_wrong_email() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();
        let invited = s
            .create_invite(&org.id, "bob@example.com", OrgRole::Member, "u-alice")
            .unwrap();
        let err = s
            .accept_invite(&invited.token, "u-bob", "charlie@example.com")
            .unwrap_err();
        assert_eq!(err, AcceptError::EmailMismatch);
    }

    #[test]
    fn invite_accept_single_use() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();
        let invited = s
            .create_invite(&org.id, "bob@example.com", OrgRole::Member, "u-alice")
            .unwrap();
        s.accept_invite(&invited.token, "u-bob", "bob@example.com")
            .unwrap();
        // Second accept of the same token is rejected.
        let err = s
            .accept_invite(&invited.token, "u-charlie", "bob@example.com")
            .unwrap_err();
        assert_eq!(err, AcceptError::AlreadyAccepted);
    }

    #[test]
    fn invite_unknown_token() {
        let s = store();
        let _ = s.create("Acme", "u-alice").unwrap();
        let err = s
            .accept_invite("not-a-real-token", "u-bob", "bob@example.com")
            .unwrap_err();
        assert_eq!(err, AcceptError::NotFound);
    }

    #[test]
    fn remove_member_clears_role() {
        let s = store();
        let org = s.create("Acme", "u-alice").unwrap();
        s.add_member(&org.id, "u-bob", OrgRole::Member);
        assert!(s.remove_member(&org.id, "u-bob"));
        assert_eq!(s.role_of(&org.id, "u-bob"), None);
    }

    #[test]
    fn disabled_config_short_circuits_every_op() {
        let store: Arc<dyn DataStore> = InMemStore::new();
        let s = OrgStore::new(
            Arc::clone(&store),
            ManifestAuthOrgConfig {
                disabled: true,
                ..Default::default()
            },
        );
        assert!(s.create("Acme", "u-alice").is_none());
        assert!(s.list_for_user("u-alice").is_empty());
        assert!(s.list_members("doesnt-matter").is_empty());
    }
}
