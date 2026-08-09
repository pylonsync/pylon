//! Studio operators — the people who run a Pylon, as distinct from the people
//! who use the app it serves.
//!
//! # Why a separate identity at all
//!
//! Studio access used to be `PYLON_ADMIN_TOKEN`: a shared secret pasted into a
//! browser form. A secret names nobody, so a destructive row edit had no one
//! to attribute it to, and it lived in `localStorage` where any XSS on the
//! origin could read it.
//!
//! Replacing it with "an admin user of the app" (`auth.user.adminField`,
//! `PYLON_ADMIN_EMAILS`) covers apps that have users and an admin among them.
//! It leaves a real gap: an API-only backend, a brand-new project, or an app
//! whose auth is OAuth-only with no admin has no way in at all. Operators
//! close that gap, and they are the more honest model for an ops tool anyway —
//! whoever operates a deployment is not necessarily a user of it.
//!
//! Both paths stay. An app that wants ops access tied to app accounts keeps
//! it; an app with no accounts to tie to gets operators.
//!
//! # Storage
//!
//! An operator is an [`Account`] row with `provider_id = "pylon_operator"`,
//! `account_id` = the username, and `password` = an Argon2 hash. That store is
//! framework-internal (`_pylon_accounts`), already persists on both SQLite and
//! Postgres, and is already wired into the runtime — so operators work on a
//! Fly/Postgres deployment the day this lands, with no new table and no
//! migration. Nothing is written to the app's own schema, which matters: an
//! ops credential should survive the app migrating or truncating its users.
//!
//! The synthetic `user_id` (`op_…`) is what a session carries. It deliberately
//! does not collide with an app user id, and [`is_operator_user_id`] is how
//! the Studio gate tells the two apart.

use crate::password::{self, PasswordPolicyError};
use crate::{generate_token, now_secs, Account, AccountStore};

/// `provider_id` marking an [`Account`] row as an operator credential.
pub const OPERATOR_PROVIDER: &str = "pylon_operator";

/// Prefix on an operator's synthetic user id.
const OPERATOR_USER_PREFIX: &str = "op_";

/// Longest accepted username. Not a storage limit — a bound so an absurd
/// value can't be used to bloat the table or a log line.
pub const MAX_USERNAME_LEN: usize = 64;

/// A Studio operator. Never carries the password hash: callers that need to
/// check a password go through [`verify`], which is the only path that should
/// ever touch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operator {
    /// Login name, e.g. `eric`.
    pub username: String,
    /// Synthetic user id sessions are minted against (`op_…`).
    pub user_id: String,
    /// Unix epoch seconds.
    pub created_at: u64,
}

/// Why an operator write was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorError {
    /// Empty, too long, or containing characters outside the allowed set.
    InvalidUsername(String),
    /// Fails the framework's password policy (length; see [`password`]).
    WeakPassword(PasswordPolicyError),
    /// A different operator already holds this username.
    UsernameTaken,
    /// No operator by that username.
    NotFound,
}

impl std::fmt::Display for OperatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUsername(why) => write!(f, "invalid username: {why}"),
            Self::WeakPassword(e) => write!(f, "{e}"),
            Self::UsernameTaken => write!(f, "an operator with that username already exists"),
            Self::NotFound => write!(f, "no such operator"),
        }
    }
}

impl std::error::Error for OperatorError {}

/// Is this user id an operator's rather than an app user's?
///
/// Cheap and total — it does not hit the store. Used on the hot path to decide
/// whether a session is worth an operator lookup at all.
pub fn is_operator_user_id(user_id: &str) -> bool {
    user_id.starts_with(OPERATOR_USER_PREFIX)
}

/// Validate a username.
///
/// Restricted to ASCII letters, digits, `.`, `-` and `_`. Deliberately narrow:
/// usernames land in URLs, log lines and shell arguments, and Unicode
/// look-alikes let two operators render identically in an audit trail while
/// being different rows.
pub fn validate_username(username: &str) -> Result<(), OperatorError> {
    if username.is_empty() {
        return Err(OperatorError::InvalidUsername("must not be empty".into()));
    }
    if username.len() > MAX_USERNAME_LEN {
        return Err(OperatorError::InvalidUsername(format!(
            "longer than {MAX_USERNAME_LEN} characters"
        )));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(OperatorError::InvalidUsername(
            "only letters, digits, '.', '-' and '_' are allowed".into(),
        ));
    }
    Ok(())
}

/// Create an operator. Fails if the username is taken — this never silently
/// overwrites an existing operator's password, which an upsert would.
pub fn create(
    store: &AccountStore,
    username: &str,
    password: &str,
) -> Result<Operator, OperatorError> {
    validate_username(username)?;
    password::validate_length(password).map_err(OperatorError::WeakPassword)?;
    if store
        .find_by_provider(OPERATOR_PROVIDER, username)
        .is_some()
    {
        return Err(OperatorError::UsernameTaken);
    }
    let now = now_secs();
    let user_id = format!("{OPERATOR_USER_PREFIX}{}", generate_token());
    store.upsert(&account_for(
        &user_id,
        username,
        &password::hash_password(password),
        now,
        now,
    ));
    Ok(Operator {
        username: username.to_string(),
        user_id,
        created_at: now,
    })
}

/// Check a username/password pair. `None` on any failure — a caller must not
/// be able to tell "no such operator" from "wrong password".
///
/// An unknown username still pays for one Argon2 verification against a dummy
/// hash. Without that, a missing user answers in microseconds and a real one
/// in ~50ms, which enumerates the operator list by clock alone.
pub fn verify(store: &AccountStore, username: &str, password: &str) -> Option<Operator> {
    let found = store.find_by_provider(OPERATOR_PROVIDER, username);
    let hash: String = found
        .as_ref()
        .and_then(|a| a.password.clone())
        .unwrap_or_else(|| password::dummy_hash().to_string());
    let ok = password::verify_password(password, &hash);
    // `ok` can only be true for the dummy hash if someone guesses the throwaway
    // password it was built from, so gate on a real row as well rather than
    // trust that alone. Same for a row with no hash at all.
    match found {
        Some(account) if ok && account.password.is_some() => Some(to_operator(&account)),
        _ => None,
    }
}

/// Replace an operator's password. Keeps the same `user_id`, so existing
/// sessions still resolve to the same identity — callers that want to kick
/// those sessions out should revoke them explicitly.
pub fn set_password(
    store: &AccountStore,
    username: &str,
    password: &str,
) -> Result<(), OperatorError> {
    password::validate_length(password).map_err(OperatorError::WeakPassword)?;
    let existing = store
        .find_by_provider(OPERATOR_PROVIDER, username)
        .ok_or(OperatorError::NotFound)?;
    store.upsert(&account_for(
        &existing.user_id,
        username,
        &password::hash_password(password),
        existing.created_at,
        now_secs(),
    ));
    Ok(())
}

/// Remove an operator. Returns the removed record so the caller can revoke its
/// sessions — deleting the credential alone would leave a live cookie working
/// until it expired.
pub fn delete(store: &AccountStore, username: &str) -> Result<Operator, OperatorError> {
    let existing = store
        .find_by_provider(OPERATOR_PROVIDER, username)
        .ok_or(OperatorError::NotFound)?;
    store.unlink(OPERATOR_PROVIDER, username);
    Ok(to_operator(&existing))
}

/// Every operator, oldest first.
pub fn list(store: &AccountStore) -> Vec<Operator> {
    let mut ops: Vec<Operator> = store
        .list_all_unfiltered()
        .iter()
        .filter(|a| a.provider_id == OPERATOR_PROVIDER)
        .map(to_operator)
        .collect();
    ops.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then(a.username.cmp(&b.username))
    });
    ops
}

/// Resolve the operator a session's `user_id` belongs to.
pub fn find_by_user_id(store: &AccountStore, user_id: &str) -> Option<Operator> {
    if !is_operator_user_id(user_id) {
        return None;
    }
    store
        .find_for_user(user_id)
        .into_iter()
        .find(|a| a.provider_id == OPERATOR_PROVIDER)
        .map(|a| to_operator(&a))
}

fn to_operator(account: &Account) -> Operator {
    Operator {
        username: account.account_id.clone(),
        user_id: account.user_id.clone(),
        created_at: account.created_at,
    }
}

fn account_for(
    user_id: &str,
    username: &str,
    password_hash: &str,
    created_at: u64,
    updated_at: u64,
) -> Account {
    Account {
        id: format!("{OPERATOR_PROVIDER}:{username}"),
        user_id: user_id.to_string(),
        provider_id: OPERATOR_PROVIDER.to_string(),
        account_id: username.to_string(),
        access_token: None,
        refresh_token: None,
        id_token: None,
        access_token_expires_at: None,
        refresh_token_expires_at: None,
        scope: None,
        password: Some(password_hash.to_string()),
        avatar_url: None,
        created_at,
        updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> AccountStore {
        AccountStore::new()
    }

    const GOOD: &str = "correct-horse-battery";

    #[test]
    fn create_then_verify_round_trips() {
        let s = store();
        let op = create(&s, "eric", GOOD).unwrap();
        assert!(is_operator_user_id(&op.user_id));
        assert_eq!(
            verify(&s, "eric", GOOD).map(|o| o.user_id),
            Some(op.user_id)
        );
    }

    #[test]
    fn the_password_is_not_stored_in_the_clear() {
        let s = store();
        create(&s, "eric", GOOD).unwrap();
        let row = s.find_by_provider(OPERATOR_PROVIDER, "eric").unwrap();
        let hash = row.password.unwrap();
        assert!(!hash.contains(GOOD), "password stored in the clear");
        assert!(hash.starts_with("$argon2"), "expected an argon2 PHC hash");
    }

    #[test]
    fn a_wrong_password_is_refused() {
        let s = store();
        create(&s, "eric", GOOD).unwrap();
        assert!(verify(&s, "eric", "not-the-password").is_none());
    }

    #[test]
    fn an_unknown_operator_is_refused() {
        let s = store();
        assert!(verify(&s, "nobody", GOOD).is_none());
    }

    #[test]
    fn an_operator_with_no_password_hash_cannot_log_in() {
        // A row hand-written without a hash (or an OAuth row that borrowed the
        // provider name) must not fall through to the dummy hash and pass.
        let s = store();
        let mut a = account_for("op_x", "ghost", "", 0, 0);
        a.password = None;
        s.upsert(&a);
        assert!(verify(&s, "ghost", "").is_none());
        assert!(verify(&s, "ghost", GOOD).is_none());
    }

    #[test]
    fn create_refuses_to_clobber_an_existing_operator() {
        // An upsert here would silently reset a colleague's password, which is
        // account takeover dressed up as a typo.
        let s = store();
        create(&s, "eric", GOOD).unwrap();
        assert_eq!(
            create(&s, "eric", "another-long-password"),
            Err(OperatorError::UsernameTaken)
        );
        // The original password still works.
        assert!(verify(&s, "eric", GOOD).is_some());
    }

    #[test]
    fn set_password_replaces_the_old_one_and_keeps_the_identity() {
        let s = store();
        let created = create(&s, "eric", GOOD).unwrap();
        set_password(&s, "eric", "a-brand-new-password").unwrap();
        assert!(
            verify(&s, "eric", GOOD).is_none(),
            "old password still works"
        );
        let after = verify(&s, "eric", "a-brand-new-password").unwrap();
        assert_eq!(after.user_id, created.user_id, "identity must be stable");
    }

    #[test]
    fn set_password_on_a_missing_operator_does_not_create_one() {
        let s = store();
        assert_eq!(
            set_password(&s, "ghost", GOOD),
            Err(OperatorError::NotFound)
        );
        assert!(verify(&s, "ghost", GOOD).is_none());
    }

    #[test]
    fn delete_removes_the_credential() {
        let s = store();
        create(&s, "eric", GOOD).unwrap();
        assert!(delete(&s, "eric").is_ok());
        assert!(verify(&s, "eric", GOOD).is_none());
        assert_eq!(delete(&s, "eric"), Err(OperatorError::NotFound));
    }

    #[test]
    fn weak_passwords_are_refused_on_create_and_change() {
        let s = store();
        assert!(matches!(
            create(&s, "eric", "short"),
            Err(OperatorError::WeakPassword(_))
        ));
        create(&s, "eric", GOOD).unwrap();
        assert!(matches!(
            set_password(&s, "eric", "short"),
            Err(OperatorError::WeakPassword(_))
        ));
        assert!(
            verify(&s, "eric", GOOD).is_some(),
            "weak change took effect"
        );
    }

    #[test]
    fn usernames_are_restricted() {
        for bad in ["", "has space", "sláinte", "a/b", "e@x.test", "semi;colon"] {
            assert!(
                validate_username(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
        for good in ["eric", "eric.campbell", "ops-1", "a_b", "A1"] {
            assert!(
                validate_username(good).is_ok(),
                "{good:?} should be allowed"
            );
        }
        assert!(validate_username(&"a".repeat(MAX_USERNAME_LEN + 1)).is_err());
    }

    #[test]
    fn find_by_user_id_resolves_only_operators() {
        let s = store();
        let op = create(&s, "eric", GOOD).unwrap();
        assert_eq!(
            find_by_user_id(&s, &op.user_id).map(|o| o.username),
            Some("eric".to_string())
        );
        // An app user id must never resolve to an operator, even by accident.
        assert!(find_by_user_id(&s, "usr_123").is_none());
        assert!(!is_operator_user_id("usr_123"));
    }

    #[test]
    fn list_returns_only_operator_rows() {
        // The accounts table also holds OAuth links. Listing operators must not
        // surface a user's Google account as an ops credential.
        let s = store();
        create(&s, "eric", GOOD).unwrap();
        create(&s, "ops-bot", GOOD).unwrap();
        s.upsert(&Account {
            id: "google:1".into(),
            user_id: "usr_1".into(),
            provider_id: "google".into(),
            account_id: "google-sub-1".into(),
            access_token: None,
            refresh_token: None,
            id_token: None,
            access_token_expires_at: None,
            refresh_token_expires_at: None,
            scope: None,
            password: None,
            avatar_url: None,
            created_at: 0,
            updated_at: 0,
        });

        let names: Vec<String> = list(&s).into_iter().map(|o| o.username).collect();
        assert_eq!(
            names.len(),
            2,
            "expected exactly the operators, got {names:?}"
        );
        assert!(names.contains(&"eric".to_string()));
        assert!(names.contains(&"ops-bot".to_string()));
    }

    #[test]
    fn an_oauth_row_cannot_be_used_as_an_operator_login() {
        // Same guard from the other direction: a google account whose
        // account_id happens to match a username must not authenticate.
        let s = store();
        s.upsert(&Account {
            id: "google:eric".into(),
            user_id: "usr_1".into(),
            provider_id: "google".into(),
            account_id: "eric".into(),
            access_token: None,
            refresh_token: None,
            id_token: None,
            access_token_expires_at: None,
            refresh_token_expires_at: None,
            scope: None,
            password: Some(password::hash_password(GOOD)),
            avatar_url: None,
            created_at: 0,
            updated_at: 0,
        });
        assert!(
            verify(&s, "eric", GOOD).is_none(),
            "lookup must be scoped to the operator provider"
        );
    }
}
