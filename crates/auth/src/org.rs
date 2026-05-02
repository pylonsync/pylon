//! Organizations + memberships + invites — multi-tenant team management.
//!
//! Sits alongside the existing in-memory `OrganizationsPlugin` in
//! `pylon_plugin::builtin::organizations` but with:
//!   - Pluggable [`OrgBackend`] trait (in-memory default, SQLite + PG
//!     backends in pylon-runtime so orgs survive a restart)
//!   - Email invite flow with token + expiry + accept endpoint
//!   - Role enforcement helpers
//!
//! The HTTP endpoints in `routes/auth.rs` use this directly. Apps
//! that want their own org model can ignore the store and roll their
//! own — pylon doesn't force the schema, only ships the backend +
//! endpoints when you opt in.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

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
    /// User id of whoever created the org. Distinct from "owner" —
    /// ownership can be transferred but creator is immutable.
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
    /// Stable id — `inv_<24-char-base64url>`. What you reference in
    /// management UIs (revoke, resend).
    pub id: String,
    pub org_id: String,
    /// Email of the invitee. Lowercased before storage so case-only
    /// duplicates collapse.
    pub email: String,
    /// Role the invitee will receive on accept.
    pub role: OrgRole,
    /// User id of whoever sent the invite. Used in the email body
    /// ("Alice invited you to Acme Corp").
    pub invited_by: String,
    /// Single-use random token — what the invitee clicks. Stored
    /// hashed (Argon2) so a DB read doesn't leak active invites.
    /// The plaintext is sent in the email and never persisted.
    pub token_hash: String,
    /// First 8 chars of the plaintext token — display in management
    /// UIs so the inviter can identify which link they sent.
    pub token_prefix: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub accepted_at: Option<u64>,
}

pub trait OrgBackend: Send + Sync {
    fn put_org(&self, org: &Org);
    fn get_org(&self, id: &str) -> Option<Org>;
    fn delete_org(&self, id: &str) -> bool;
    fn list_orgs_for_user(&self, user_id: &str) -> Vec<(Org, OrgRole)>;

    fn put_membership(&self, m: &Membership);
    fn get_membership(&self, org_id: &str, user_id: &str) -> Option<Membership>;
    fn delete_membership(&self, org_id: &str, user_id: &str) -> bool;
    fn list_members(&self, org_id: &str) -> Vec<Membership>;

    fn put_invite(&self, inv: &Invite);
    fn get_invite(&self, id: &str) -> Option<Invite>;
    fn list_invites(&self, org_id: &str) -> Vec<Invite>;
    fn delete_invite(&self, id: &str) -> bool;
    /// All non-accepted invites whose plaintext starts with `prefix`.
    /// SQL backends use a `WHERE token_prefix = $1 AND accepted_at IS NULL`
    /// SELECT; the in-memory backend scans all invites. Argon2 verify
    /// then runs against the candidate set in `accept_invite`.
    fn invites_by_prefix(&self, prefix: &str) -> Vec<Invite>;
    /// CAS — atomically stamp `accepted_at` ONLY when it's currently
    /// NULL. Returns true if we won the race, false if another
    /// concurrent verify got there first. Required so two parallel
    /// accept calls with the same token can't BOTH create a
    /// membership.
    fn mark_invite_accepted(&self, id: &str, now: u64) -> bool;
}

pub struct InMemoryOrgBackend {
    orgs: Mutex<HashMap<String, Org>>,
    memberships: Mutex<HashMap<(String, String), Membership>>,
    invites: Mutex<HashMap<String, Invite>>,
}

impl Default for InMemoryOrgBackend {
    fn default() -> Self {
        Self {
            orgs: Mutex::new(HashMap::new()),
            memberships: Mutex::new(HashMap::new()),
            invites: Mutex::new(HashMap::new()),
        }
    }
}

impl OrgBackend for InMemoryOrgBackend {
    fn put_org(&self, org: &Org) {
        self.orgs.lock().unwrap().insert(org.id.clone(), org.clone());
    }
    fn get_org(&self, id: &str) -> Option<Org> {
        self.orgs.lock().unwrap().get(id).cloned()
    }
    fn delete_org(&self, id: &str) -> bool {
        let removed = self.orgs.lock().unwrap().remove(id).is_some();
        if removed {
            self.memberships
                .lock()
                .unwrap()
                .retain(|(o, _), _| o != id);
            self.invites
                .lock()
                .unwrap()
                .retain(|_, inv| inv.org_id != id);
        }
        removed
    }
    fn list_orgs_for_user(&self, user_id: &str) -> Vec<(Org, OrgRole)> {
        let m = self.memberships.lock().unwrap();
        let o = self.orgs.lock().unwrap();
        m.values()
            .filter(|mem| mem.user_id == user_id)
            .filter_map(|mem| o.get(&mem.org_id).map(|org| (org.clone(), mem.role)))
            .collect()
    }

    fn put_membership(&self, m: &Membership) {
        self.memberships
            .lock()
            .unwrap()
            .insert((m.org_id.clone(), m.user_id.clone()), m.clone());
    }
    fn get_membership(&self, org_id: &str, user_id: &str) -> Option<Membership> {
        self.memberships
            .lock()
            .unwrap()
            .get(&(org_id.to_string(), user_id.to_string()))
            .cloned()
    }
    fn delete_membership(&self, org_id: &str, user_id: &str) -> bool {
        self.memberships
            .lock()
            .unwrap()
            .remove(&(org_id.to_string(), user_id.to_string()))
            .is_some()
    }
    fn list_members(&self, org_id: &str) -> Vec<Membership> {
        self.memberships
            .lock()
            .unwrap()
            .values()
            .filter(|m| m.org_id == org_id)
            .cloned()
            .collect()
    }

    fn put_invite(&self, inv: &Invite) {
        self.invites
            .lock()
            .unwrap()
            .insert(inv.id.clone(), inv.clone());
    }
    fn get_invite(&self, id: &str) -> Option<Invite> {
        self.invites.lock().unwrap().get(id).cloned()
    }
    fn list_invites(&self, org_id: &str) -> Vec<Invite> {
        self.invites
            .lock()
            .unwrap()
            .values()
            .filter(|i| i.org_id == org_id && i.accepted_at.is_none())
            .cloned()
            .collect()
    }
    fn delete_invite(&self, id: &str) -> bool {
        self.invites.lock().unwrap().remove(id).is_some()
    }
    fn invites_by_prefix(&self, prefix: &str) -> Vec<Invite> {
        // Include accepted invites in the candidate set so the
        // accept path can return `AlreadyAccepted` (good UX) instead
        // of `NotFound` (confusing — looks like a typo in the link).
        self.invites
            .lock()
            .unwrap()
            .values()
            .filter(|i| i.token_prefix == prefix)
            .cloned()
            .collect()
    }
    fn mark_invite_accepted(&self, id: &str, now: u64) -> bool {
        let mut g = self.invites.lock().unwrap();
        let Some(inv) = g.get_mut(id) else {
            return false;
        };
        if inv.accepted_at.is_some() {
            return false;
        }
        inv.accepted_at = Some(now);
        true
    }
}

pub struct OrgStore {
    backend: Box<dyn OrgBackend>,
}

impl Default for OrgStore {
    fn default() -> Self {
        Self::new()
    }
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
    /// Token doesn't match any stored invite (typo, never sent,
    /// or revoked by an admin). Frontend should ask the user to
    /// request a fresh invite.
    NotFound,
    /// Invite is past `expires_at`. Frontend should ask for a resend.
    Expired,
    /// Invite was already redeemed by SOMEONE (possibly the same
    /// user, possibly a different account that shared the email).
    /// **Frontends should treat this as success** for UX — the user
    /// is effectively in the org via that prior accept; surface as
    /// "you're already a member" not as an error.
    AlreadyAccepted,
    /// The accepting user's email doesn't match the invite's
    /// addressee. This is the security gate — surface as a real
    /// error ("this invite was sent to <other-email>; sign in
    /// with that account to accept").
    EmailMismatch,
    /// User is already a member of this org via a DIFFERENT path
    /// (e.g. they created the org themselves, or accepted an earlier
    /// invite). **Frontends should treat this as success** — the
    /// invite was redundant.
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

impl OrgStore {
    pub fn new() -> Self {
        Self::with_backend(Box::new(InMemoryOrgBackend::default()))
    }

    pub fn with_backend(backend: Box<dyn OrgBackend>) -> Self {
        Self { backend }
    }

    /// Create an org. Creator becomes Owner.
    pub fn create(&self, name: &str, creator_id: &str) -> Org {
        let id = format!("org_{}", random_token(20));
        let org = Org {
            id: id.clone(),
            name: name.to_string(),
            created_by: creator_id.to_string(),
            created_at: now_secs(),
        };
        self.backend.put_org(&org);
        self.backend.put_membership(&Membership {
            org_id: id,
            user_id: creator_id.to_string(),
            role: OrgRole::Owner,
            joined_at: now_secs(),
        });
        org
    }

    pub fn get(&self, org_id: &str) -> Option<Org> {
        self.backend.get_org(org_id)
    }

    pub fn list_for_user(&self, user_id: &str) -> Vec<(Org, OrgRole)> {
        self.backend.list_orgs_for_user(user_id)
    }

    pub fn list_members(&self, org_id: &str) -> Vec<Membership> {
        self.backend.list_members(org_id)
    }

    pub fn role_of(&self, org_id: &str, user_id: &str) -> Option<OrgRole> {
        self.backend.get_membership(org_id, user_id).map(|m| m.role)
    }

    pub fn set_role(&self, org_id: &str, user_id: &str, role: OrgRole) -> bool {
        if let Some(mut m) = self.backend.get_membership(org_id, user_id) {
            m.role = role;
            self.backend.put_membership(&m);
            true
        } else {
            false
        }
    }

    pub fn remove_member(&self, org_id: &str, user_id: &str) -> bool {
        self.backend.delete_membership(org_id, user_id)
    }

    /// Delete an org + all its memberships + all pending invites.
    pub fn delete(&self, org_id: &str) -> bool {
        self.backend.delete_org(org_id)
    }

    /// Mint an invite. Returns the plaintext token alongside the
    /// stored record — caller is responsible for emailing the
    /// plaintext to the invitee. The token is single-use, expires
    /// in 7 days, and is rejected for any account whose email
    /// doesn't match the invite's `email` field.
    pub fn create_invite(
        &self,
        org_id: &str,
        email: &str,
        role: OrgRole,
        invited_by: &str,
    ) -> InviteWithToken {
        let id = format!("inv_{}", random_token(20));
        let token = random_token(24);
        let token_hash = crate::password::hash_password(&token);
        let token_prefix: String = token.chars().take(8).collect();
        let expires_at = now_secs() + 7 * 24 * 60 * 60; // 7 days
        let invite = Invite {
            id,
            org_id: org_id.to_string(),
            email: email.to_lowercase(),
            role,
            invited_by: invited_by.to_string(),
            token_hash,
            token_prefix,
            created_at: now_secs(),
            expires_at,
            accepted_at: None,
        };
        self.backend.put_invite(&invite);
        InviteWithToken { invite, token }
    }

    pub fn list_invites(&self, org_id: &str) -> Vec<Invite> {
        self.backend.list_invites(org_id)
    }

    pub fn revoke_invite(&self, invite_id: &str) -> bool {
        self.backend.delete_invite(invite_id)
    }

    /// Accept an invite. Verifies the token (Argon2 hash compare),
    /// checks expiry + accepted-at, ensures the accepting user's
    /// email matches the invite, and either creates the membership
    /// or returns the right error variant. The invite row is
    /// updated with `accepted_at` (not deleted) so the audit trail
    /// stays intact.
    pub fn accept_invite(
        &self,
        token: &str,
        accepting_user_id: &str,
        accepting_email: &str,
    ) -> Result<Membership, AcceptError> {
        // Linear scan for the matching token hash. At org-management
        // scale (handfuls of pending invites per org) this is fine;
        // an index by token-hash-prefix would help if it ever wasn't.
        // We can't store the token directly because that would let a
        // DB read hand attackers active invite links.
        let invite = self
            .find_invite_by_plaintext(token)
            .ok_or(AcceptError::NotFound)?;
        if invite.accepted_at.is_some() {
            return Err(AcceptError::AlreadyAccepted);
        }
        if invite.expires_at <= now_secs() {
            return Err(AcceptError::Expired);
        }
        if invite.email != accepting_email.to_lowercase() {
            return Err(AcceptError::EmailMismatch);
        }
        if self
            .backend
            .get_membership(&invite.org_id, accepting_user_id)
            .is_some()
        {
            return Err(AcceptError::AlreadyMember);
        }
        // Wave-4 codex P2: CAS the invite to accepted_at FIRST,
        // BEFORE creating the membership. If two concurrent accepts
        // arrive, only one wins the CAS and only one membership
        // gets created. The loser sees AlreadyAccepted (the invite
        // was just consumed by the winning request).
        if !self.backend.mark_invite_accepted(&invite.id, now_secs()) {
            return Err(AcceptError::AlreadyAccepted);
        }
        let membership = Membership {
            org_id: invite.org_id.clone(),
            user_id: accepting_user_id.to_string(),
            role: invite.role,
            joined_at: now_secs(),
        };
        self.backend.put_membership(&membership);
        Ok(membership)
    }

    /// Resolve a plaintext invite token to its stored record.
    /// Narrows by `token_prefix` (cheap SQL index lookup) then
    /// Argon2-verifies the candidate set. Argon2 is non-deterministic
    /// so we can't direct-lookup by hash — but invitations live for
    /// 7 days max and prefix collisions are 64 bits → effectively 1
    /// candidate per query in practice.
    fn find_invite_by_plaintext(&self, token: &str) -> Option<Invite> {
        let prefix: String = token.chars().take(8).collect();
        for inv in self.backend.invites_by_prefix(&prefix) {
            if crate::password::verify_password(token, &inv.token_hash) {
                return Some(inv);
            }
        }
        None
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
    fn create_org_makes_creator_owner() {
        let store = OrgStore::new();
        let org = store.create("Acme", "user-1");
        assert!(org.id.starts_with("org_"));
        assert_eq!(org.name, "Acme");
        assert_eq!(store.role_of(&org.id, "user-1"), Some(OrgRole::Owner));
    }

    #[test]
    fn list_for_user_returns_all_orgs() {
        let store = OrgStore::new();
        let a = store.create("A", "u1");
        let _b = store.create("B", "u2");
        let c = store.create("C", "u3");
        store.set_role(&c.id, "u1", OrgRole::Member);
        // u1 owns A and isn't in C yet — set_role only updates an
        // existing membership, so add it via the backend.
        store
            .backend
            .put_membership(&Membership {
                org_id: c.id.clone(),
                user_id: "u1".into(),
                role: OrgRole::Member,
                joined_at: 1,
            });
        let list = store.list_for_user("u1");
        assert_eq!(list.len(), 2);
        let names: Vec<_> = list.iter().map(|(o, _)| o.name.clone()).collect();
        assert!(names.contains(&"A".to_string()));
        assert!(names.contains(&"C".to_string()));
        assert!(!names.contains(&"B".to_string()));
    }

    #[test]
    fn role_helpers() {
        assert!(OrgRole::Owner.can_manage_members());
        assert!(OrgRole::Owner.can_delete_org());
        assert!(OrgRole::Admin.can_manage_members());
        assert!(!OrgRole::Admin.can_delete_org());
        assert!(!OrgRole::Member.can_manage_members());
    }

    #[test]
    fn delete_cascades_memberships_and_invites() {
        let store = OrgStore::new();
        let org = store.create("A", "owner-1");
        let _inv = store.create_invite(&org.id, "x@example.com", OrgRole::Member, "owner-1");
        assert_eq!(store.list_invites(&org.id).len(), 1);
        assert_eq!(store.list_members(&org.id).len(), 1);
        assert!(store.delete(&org.id));
        assert!(store.get(&org.id).is_none());
        assert!(store.list_members(&org.id).is_empty());
        assert!(store.list_invites(&org.id).is_empty());
    }

    #[test]
    fn accept_invite_creates_membership() {
        let store = OrgStore::new();
        let org = store.create("Acme", "owner-1");
        let invited = store.create_invite(
            &org.id,
            "newbie@example.com",
            OrgRole::Admin,
            "owner-1",
        );
        let m = store
            .accept_invite(&invited.token, "user-2", "newbie@example.com")
            .expect("accept");
        assert_eq!(m.role, OrgRole::Admin);
        assert_eq!(store.role_of(&org.id, "user-2"), Some(OrgRole::Admin));
        // Audit: invite stamped accepted, not deleted.
        let stored = store.backend.get_invite(&invited.invite.id).unwrap();
        assert!(stored.accepted_at.is_some());
    }

    #[test]
    fn accept_invite_rejects_wrong_email() {
        let store = OrgStore::new();
        let org = store.create("Acme", "owner-1");
        let invited =
            store.create_invite(&org.id, "alice@example.com", OrgRole::Member, "owner-1");
        let err = store
            .accept_invite(&invited.token, "user-2", "bob@example.com")
            .unwrap_err();
        assert_eq!(err, AcceptError::EmailMismatch);
    }

    #[test]
    fn accept_invite_rejects_replay() {
        let store = OrgStore::new();
        let org = store.create("A", "owner");
        let invited = store.create_invite(&org.id, "a@b.com", OrgRole::Member, "owner");
        store
            .accept_invite(&invited.token, "user-2", "a@b.com")
            .unwrap();
        let second = store.accept_invite(&invited.token, "user-2", "a@b.com");
        assert_eq!(second.unwrap_err(), AcceptError::AlreadyAccepted);
    }

    /// Wave-4 codex P2 regression: concurrent accepts must not
    /// both create a membership. The CAS via `mark_invite_accepted`
    /// guarantees only one wins. Simulate by calling
    /// `mark_invite_accepted` twice — the second call must return
    /// false so the second accept_invite returns AlreadyAccepted.
    #[test]
    fn accept_invite_cas_blocks_concurrent_winners() {
        let store = OrgStore::new();
        let org = store.create("A", "owner");
        let invited = store.create_invite(&org.id, "a@b.com", OrgRole::Member, "owner");

        // Simulate: first request gets through to mark_invite_accepted
        // and wins.
        let won_first = store.backend.mark_invite_accepted(&invited.invite.id, 100);
        assert!(won_first);
        // Second concurrent request runs the same CAS and loses.
        let won_second = store.backend.mark_invite_accepted(&invited.invite.id, 101);
        assert!(!won_second);
        // accept_invite called now would see consumed_at set and
        // return AlreadyAccepted instead of double-creating.
        let result = store.accept_invite(&invited.token, "user-x", "a@b.com");
        assert_eq!(result.unwrap_err(), AcceptError::AlreadyAccepted);
    }

    #[test]
    fn accept_invite_rejects_unknown_token() {
        let store = OrgStore::new();
        let _org = store.create("A", "owner");
        let err = store
            .accept_invite("not-a-real-token", "user-2", "x@y.com")
            .unwrap_err();
        assert_eq!(err, AcceptError::NotFound);
    }

    #[test]
    fn invite_email_lowercased() {
        let store = OrgStore::new();
        let org = store.create("A", "owner");
        let inv = store.create_invite(&org.id, "Mixed@CASE.com", OrgRole::Member, "owner");
        assert_eq!(inv.invite.email, "mixed@case.com");
    }

    #[test]
    fn revoke_invite() {
        let store = OrgStore::new();
        let org = store.create("A", "owner");
        let inv = store.create_invite(&org.id, "x@y.com", OrgRole::Member, "owner");
        assert!(store.revoke_invite(&inv.invite.id));
        assert!(store.list_invites(&org.id).is_empty());
    }

    #[test]
    fn remove_member() {
        let store = OrgStore::new();
        let org = store.create("A", "owner");
        store.backend.put_membership(&Membership {
            org_id: org.id.clone(),
            user_id: "u2".into(),
            role: OrgRole::Member,
            joined_at: 1,
        });
        assert!(store.remove_member(&org.id, "u2"));
        assert!(store.role_of(&org.id, "u2").is_none());
    }
}
