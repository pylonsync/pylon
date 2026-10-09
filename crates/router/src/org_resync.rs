//! Re-run the federated org mirror for a signed-in user.
//!
//! The login path (`complete_oauth_login_pkce`) mirrors the IdP's `orgs`
//! claim into local Org memberships. Without this module that was the only
//! time it ran, so an org created upstream after sign-in stayed invisible
//! until the user signed out and back in. Here the stored Account token
//! fetches fresh userinfo and the same mirror runs again:
//!
//!   - from `GET /api/auth/me`, in a background thread, at most once per
//!     [`AUTO_RESYNC_INTERVAL`] per user (a login counts as a run);
//!   - from `POST /api/auth/orgs/refresh`, inline, at most once per
//!     [`MANUAL_RESYNC_INTERVAL`] per user.
//!
//! A missing `orgs` claim never changes memberships. Only a claim the IdP
//! actually sent is mirrored; treating a failed fetch as "no orgs" would
//! remove every mirrored membership under `remove_missing`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pylon_auth::org::OrgStore;
use pylon_auth::org_federation::{mirror_external_orgs, MirrorReport};
use pylon_auth::{AccountStore, ExternalOrg, RefreshError};
use pylon_kernel::ManifestAuthOrgFederation;

use crate::RouterContext;

/// Minimum time between automatic resyncs for one user.
pub const AUTO_RESYNC_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Minimum time between explicit `POST /api/auth/orgs/refresh` runs for
/// one user. Each run costs the IdP a userinfo call.
pub const MANUAL_RESYNC_INTERVAL: Duration = Duration::from_secs(10);

/// Refresh the access token when it expires within this many seconds,
/// so it does not expire between the check and the userinfo call.
const TOKEN_REFRESH_BUFFER_SECS: u64 = 60;

/// Throttle maps prune stale entries once they hold this many users.
const THROTTLE_PRUNE_AT: usize = 10_000;

/// Why a resync did not run. `status` and `code` are the HTTP status and
/// error code `POST /api/auth/orgs/refresh` answers with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResyncError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl ResyncError {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ResyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// Re-run the federation mirror for `user_id` with a fresh `orgs` claim
/// from the IdP. A no-op when the app has no `auth.org.federation`.
///
/// Errors (no linked account, token refresh failed, userinfo failed)
/// leave memberships unchanged.
pub fn resync_federated_orgs(
    ctx: &RouterContext,
    user_id: &str,
) -> Result<MirrorReport, ResyncError> {
    resync_with(ctx.orgs, ctx.account_store, user_id)
}

/// [`resync_federated_orgs`] against explicit stores, for callers that
/// run off the request path and so hold no `RouterContext`.
pub fn resync_with(
    orgs: &OrgStore,
    accounts: &AccountStore,
    user_id: &str,
) -> Result<MirrorReport, ResyncError> {
    let Some(fed) = orgs.federation() else {
        return Ok(MirrorReport::default());
    };
    // Same match the login path uses: provider names compare without case.
    let account = accounts
        .find_for_user(user_id)
        .into_iter()
        .find(|a| a.provider_id.eq_ignore_ascii_case(&fed.provider))
        .ok_or_else(|| {
            ResyncError::new(
                404,
                "ACCOUNT_NOT_FOUND",
                format!("no linked {} account for this user", fed.provider),
            )
        })?;
    // Refreshes and persists the token set only when it is about to
    // expire. It serializes refreshes per account, which matters for
    // providers that rotate the refresh token on use.
    let account = accounts
        .ensure_fresh_access_token(
            &account.provider_id,
            &account.account_id,
            TOKEN_REFRESH_BUFFER_SECS,
        )
        .map_err(refresh_error)?;
    let access_token = account
        .access_token
        .as_deref()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            ResyncError::new(400, "NO_ACCESS_TOKEN", "linked account has no access token")
        })?;
    let config = pylon_auth::OAuthRegistry::shared()
        .get(&account.provider_id)
        .cloned()
        .ok_or_else(|| {
            ResyncError::new(
                501,
                "PROVIDER_NOT_CONFIGURED",
                format!("OAuth provider \"{}\" not configured", account.provider_id),
            )
        })?;
    let info = config
        .fetch_userinfo_with_id_token(access_token, account.id_token.as_deref())
        .map_err(|e| ResyncError::new(502, "USERINFO_FAILED", format!("userinfo: {e}")))?;
    // The token must still speak for the account it is stored on.
    if info.provider_account_id != account.account_id {
        return Err(ResyncError::new(
            502,
            "ACCOUNT_MISMATCH",
            "userinfo returned a different account than the stored link",
        ));
    }
    Ok(apply_claim(orgs, fed, user_id, info.orgs.as_deref()))
}

/// Mirror `claim` into local memberships. An absent claim means the IdP
/// did not send one (scope not granted, older IdP), not that the user has
/// no orgs, so it changes nothing and reads nothing. An empty list is a
/// real answer and is mirrored.
fn apply_claim(
    orgs: &OrgStore,
    fed: &ManifestAuthOrgFederation,
    user_id: &str,
    claim: Option<&[ExternalOrg]>,
) -> MirrorReport {
    let Some(claim) = claim else {
        return MirrorReport::default();
    };
    mirror_external_orgs(orgs, fed, orgs.declared_roles(), user_id, Some(claim))
}

fn refresh_error(err: RefreshError) -> ResyncError {
    let status = match err {
        RefreshError::AccountNotFound => 404,
        RefreshError::NoRefreshToken => 400,
        RefreshError::ProviderNotConfigured => 501,
        RefreshError::RefreshFailed(_) => 502,
    };
    ResyncError::new(status, err.code(), err.message())
}

/// True when a user last run at `last` may run again at `now`.
fn is_due(last: Option<Instant>, now: Instant, interval: Duration) -> bool {
    match last {
        Some(t) => now.saturating_duration_since(t) >= interval,
        None => true,
    }
}

/// Per-user "last ran at" map with a fixed interval.
struct Throttle {
    interval: Duration,
    last: Mutex<HashMap<String, Instant>>,
}

impl Throttle {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: Mutex::new(HashMap::new()),
        }
    }

    /// Record a run at `now` if the user is due. Returns the time left
    /// until the next allowed run when not due.
    fn try_claim(&self, user_id: &str, now: Instant) -> Result<(), Duration> {
        let mut map = self.last.lock().unwrap_or_else(|p| p.into_inner());
        let last = map.get(user_id).copied();
        if !is_due(last, now, self.interval) {
            let elapsed = last.map_or(Duration::ZERO, |t| now.saturating_duration_since(t));
            return Err(self.interval.saturating_sub(elapsed));
        }
        self.insert(&mut map, user_id, now);
        Ok(())
    }

    fn mark(&self, user_id: &str, now: Instant) {
        let mut map = self.last.lock().unwrap_or_else(|p| p.into_inner());
        self.insert(&mut map, user_id, now);
    }

    fn insert(&self, map: &mut HashMap<String, Instant>, user_id: &str, now: Instant) {
        if map.len() >= THROTTLE_PRUNE_AT {
            let interval = self.interval;
            map.retain(|_, t| !is_due(Some(*t), now, interval));
        }
        map.insert(user_id.to_string(), now);
    }
}

struct Inner {
    orgs: Arc<OrgStore>,
    accounts: Arc<AccountStore>,
    auto: Throttle,
    manual: Throttle,
    /// One resync per user at a time, so a background run and an explicit
    /// refresh never both create the same mirror.
    user_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

/// Runtime-owned handle for org resyncs. Holds owned store handles so a
/// resync can run on its own thread after the request returns, plus the
/// per-user throttles. Cheap to clone.
#[derive(Clone)]
pub struct OrgResync {
    inner: Arc<Inner>,
}

impl OrgResync {
    pub fn new(orgs: Arc<OrgStore>, accounts: Arc<AccountStore>) -> Self {
        Self {
            inner: Arc::new(Inner {
                orgs,
                accounts,
                auto: Throttle::new(AUTO_RESYNC_INTERVAL),
                manual: Throttle::new(MANUAL_RESYNC_INTERVAL),
                user_locks: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// A login just mirrored the claim; start the automatic interval now.
    pub fn record_login(&self, user_id: &str) {
        self.inner.auto.mark(user_id, Instant::now());
    }

    /// Start a background resync for `user_id` unless one ran within
    /// [`AUTO_RESYNC_INTERVAL`]. Returns at once; errors are logged.
    pub fn spawn_if_due(&self, user_id: &str) {
        if self.inner.orgs.federation().is_none() {
            return;
        }
        if self.inner.auto.try_claim(user_id, Instant::now()).is_err() {
            return;
        }
        let inner = Arc::clone(&self.inner);
        let uid = user_id.to_string();
        // 2 MiB like the HTTP workers: the TLS handshake to the IdP
        // overflows a smaller stack in debug builds.
        let spawned = std::thread::Builder::new()
            .name("pylon-org-resync".into())
            .stack_size(2 * 1024 * 1024)
            .spawn(move || match inner.run(&uid) {
                Ok(_) => {}
                // No linked account is normal for users who signed in
                // some other way.
                Err(e) if e.code == "ACCOUNT_NOT_FOUND" => {
                    tracing::debug!("[org] federation resync skipped for user={uid}: {e}");
                }
                Err(e) => {
                    tracing::warn!("[org] federation resync failed for user={uid}: {e}");
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("[org] could not start federation resync thread: {e}");
        }
    }

    /// Resync `user_id` now, at most once per [`MANUAL_RESYNC_INTERVAL`].
    /// Also restarts the automatic interval.
    pub fn run_now(&self, user_id: &str) -> Result<MirrorReport, ResyncError> {
        let now = Instant::now();
        if let Err(wait) = self.inner.manual.try_claim(user_id, now) {
            let secs = wait.as_secs().max(1);
            return Err(ResyncError::new(
                429,
                "RATE_LIMITED",
                format!("Org refresh was requested too recently. Try again in {secs}s."),
            ));
        }
        self.inner.auto.mark(user_id, now);
        self.inner.run(user_id)
    }
}

impl Inner {
    fn run(&self, user_id: &str) -> Result<MirrorReport, ResyncError> {
        let lock = {
            let mut locks = self.user_locks.lock().unwrap_or_else(|p| p.into_inner());
            // Drop locks nobody else holds so the map stays small.
            locks.retain(|_, l| Arc::strong_count(l) > 1);
            locks
                .entry(user_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        resync_with(&self.orgs, &self.accounts, user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_http::{DataError, DataStore};
    use pylon_kernel::{AppManifest, ManifestAuthOrgConfig};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fed() -> ManifestAuthOrgFederation {
        ManifestAuthOrgFederation {
            provider: "idp".into(),
            external_id_field: "externalId".into(),
            remove_missing: true,
            role_map: Default::default(),
            disable_local_create: true,
            slug_field: None,
        }
    }

    /// Empty store that counts every data call, so a test can prove a
    /// path never read or wrote memberships.
    struct CountingStore {
        manifest: AppManifest,
        calls: Arc<AtomicUsize>,
    }

    impl CountingStore {
        fn hit(&self) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl DataStore for CountingStore {
        fn manifest(&self) -> &AppManifest {
            &self.manifest
        }
        fn insert(&self, _: &str, _: &serde_json::Value) -> Result<String, DataError> {
            self.hit();
            Ok("id".into())
        }
        fn get_by_id(&self, _: &str, _: &str) -> Result<Option<serde_json::Value>, DataError> {
            self.hit();
            Ok(None)
        }
        fn list(&self, _: &str) -> Result<Vec<serde_json::Value>, DataError> {
            self.hit();
            Ok(Vec::new())
        }
        fn list_after(
            &self,
            _: &str,
            _: Option<&str>,
            _: usize,
        ) -> Result<Vec<serde_json::Value>, DataError> {
            self.hit();
            Ok(Vec::new())
        }
        fn update(&self, _: &str, _: &str, _: &serde_json::Value) -> Result<bool, DataError> {
            self.hit();
            Ok(false)
        }
        fn delete(&self, _: &str, _: &str) -> Result<bool, DataError> {
            self.hit();
            Ok(false)
        }
        fn lookup(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<Option<serde_json::Value>, DataError> {
            self.hit();
            Ok(None)
        }
        fn link(&self, _: &str, _: &str, _: &str, _: &str) -> Result<bool, DataError> {
            self.hit();
            Ok(false)
        }
        fn unlink(&self, _: &str, _: &str, _: &str) -> Result<bool, DataError> {
            self.hit();
            Ok(false)
        }
        fn query_filtered(
            &self,
            _: &str,
            _: &serde_json::Value,
        ) -> Result<Vec<serde_json::Value>, DataError> {
            self.hit();
            Ok(Vec::new())
        }
        fn query_graph(&self, _: &serde_json::Value) -> Result<serde_json::Value, DataError> {
            self.hit();
            Ok(serde_json::Value::Null)
        }
        fn transact(
            &self,
            _: &[serde_json::Value],
        ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
            self.hit();
            Ok((true, Vec::new()))
        }
    }

    fn counting_orgs(
        federation: Option<ManifestAuthOrgFederation>,
    ) -> (OrgStore, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let store: Arc<dyn DataStore> = Arc::new(CountingStore {
            manifest: AppManifest::default(),
            calls: Arc::clone(&calls),
        });
        let cfg = ManifestAuthOrgConfig {
            federation,
            ..Default::default()
        };
        (OrgStore::new(store, cfg), calls)
    }

    #[test]
    fn absent_claim_changes_nothing() {
        let (orgs, touched) = counting_orgs(Some(fed()));
        let report = apply_claim(&orgs, &fed(), "u1", None);
        assert_eq!(report, MirrorReport::default());
        assert_eq!(
            touched.load(Ordering::SeqCst),
            0,
            "no store reads or writes"
        );
    }

    #[test]
    fn empty_claim_reaches_the_mirror() {
        // An empty list is the IdP saying "no orgs"; remove_missing must
        // see it, so the mirror reads the user's memberships.
        let (orgs, touched) = counting_orgs(Some(fed()));
        let report = apply_claim(&orgs, &fed(), "u1", Some(&[]));
        assert_eq!(report, MirrorReport::default());
        assert!(touched.load(Ordering::SeqCst) > 0);
    }

    #[test]
    fn no_federation_is_a_no_op() {
        let (orgs, touched) = counting_orgs(None);
        let accounts = AccountStore::new();
        assert_eq!(
            resync_with(&orgs, &accounts, "u1"),
            Ok(MirrorReport::default())
        );
        assert_eq!(touched.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn missing_account_is_an_error_and_changes_nothing() {
        let (orgs, touched) = counting_orgs(Some(fed()));
        let accounts = AccountStore::new();
        let err = resync_with(&orgs, &accounts, "u1").unwrap_err();
        assert_eq!(err.code, "ACCOUNT_NOT_FOUND");
        assert_eq!(err.status, 404);
        assert_eq!(touched.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn is_due_respects_interval() {
        let t0 = Instant::now();
        let iv = Duration::from_secs(300);
        assert!(is_due(None, t0, iv));
        assert!(!is_due(Some(t0), t0, iv));
        assert!(!is_due(Some(t0), t0 + Duration::from_secs(299), iv));
        assert!(is_due(Some(t0), t0 + iv, iv));
        // A clock reading earlier than the stamp is never due.
        assert!(!is_due(Some(t0 + iv), t0, iv));
    }

    #[test]
    fn throttle_allows_once_per_interval_per_user() {
        let th = Throttle::new(Duration::from_secs(300));
        let t0 = Instant::now();
        assert!(th.try_claim("u1", t0).is_ok());
        let wait = th
            .try_claim("u1", t0 + Duration::from_secs(100))
            .unwrap_err();
        assert_eq!(wait, Duration::from_secs(200));
        // Another user has its own slot.
        assert!(th.try_claim("u2", t0 + Duration::from_secs(100)).is_ok());
        assert!(th.try_claim("u1", t0 + Duration::from_secs(300)).is_ok());
    }

    #[test]
    fn mark_starts_the_interval() {
        let th = Throttle::new(Duration::from_secs(300));
        let t0 = Instant::now();
        th.mark("u1", t0);
        assert!(th.try_claim("u1", t0 + Duration::from_secs(10)).is_err());
        assert!(th.try_claim("u1", t0 + Duration::from_secs(301)).is_ok());
    }

    #[test]
    fn throttle_prunes_expired_entries_when_full() {
        let th = Throttle::new(Duration::from_secs(1));
        let t0 = Instant::now();
        for i in 0..THROTTLE_PRUNE_AT {
            th.mark(&format!("u{i}"), t0);
        }
        th.mark("fresh", t0 + Duration::from_secs(2));
        let map = th.last.lock().unwrap();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("fresh"));
    }
}
