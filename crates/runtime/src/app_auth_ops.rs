//! `ctx.auth.providerToken` and `ctx.auth.verifyOidcToken`.
//!
//! An app that signs people in against an OIDC provider (a Pylon IdP such
//! as id.stack0.dev, or Google) keeps each person's provider tokens on the
//! Account row. `providerToken` hands server code a fresh access token for
//! that row, so the app can call the provider's API as the person. The
//! provider app checks such a token with `verifyOidcToken`.
//!
//! The runner has already checked the function type and who may read
//! another user's token (`auth_op_gate` in pylon-functions); this module
//! does the store work.

use pylon_auth::{Account, AccountStore, RefreshError};
use pylon_functions::protocol::AuthOpMessage;

/// Refresh when the access token has less than this many seconds left, so
/// the caller never gets a token that expires mid-request.
const REFRESH_BUFFER_SECS: u64 = 60;

/// Handle one `auth_op` frame. `user_id` is the user the runner resolved
/// for `provider_token`.
pub fn handle_op(
    accounts: &AccountStore,
    req: &AuthOpMessage,
    user_id: Option<&str>,
) -> Result<serde_json::Value, (String, String)> {
    match req.op.as_str() {
        "provider_token" => {
            let provider = req.provider.as_deref().unwrap_or("").trim();
            let user_id = user_id.unwrap_or("");
            provider_token(accounts, provider, user_id, |provider_id, account_id| {
                accounts.ensure_fresh_access_token(provider_id, account_id, REFRESH_BUFFER_SECS)
            })
        }
        "verify_oidc_token" => verify_oidc_token(req.token.as_deref().unwrap_or("")),
        other => Err((
            "INVALID_ARGS".into(),
            format!("unknown ctx.auth op: {other}"),
        )),
    }
}

/// The access token of `user_id`'s account with `provider`, refreshed
/// through `refresh` when it is about to expire. `null` when the user has
/// no account with that provider. Never returns a stale token: when a
/// refresh is needed and cannot happen, it fails.
fn provider_token(
    accounts: &AccountStore,
    provider: &str,
    user_id: &str,
    refresh: impl Fn(&str, &str) -> Result<Account, RefreshError>,
) -> Result<serde_json::Value, (String, String)> {
    // A user can link two accounts of one provider; the one signed in with
    // last is the one the person expects the app to act as.
    let Some(linked) = accounts
        .find_for_user(user_id)
        .into_iter()
        .filter(|a| a.provider_id == provider)
        .max_by_key(|a| a.updated_at)
    else {
        return Ok(serde_json::Value::Null);
    };
    let account = match refresh(&linked.provider_id, &linked.account_id) {
        Ok(a) => a,
        // Unlinked between the two reads.
        Err(RefreshError::AccountNotFound) => return Ok(serde_json::Value::Null),
        Err(e) if e.needs_reauth() => {
            return Err((
                "REAUTH_REQUIRED".into(),
                format!(
                    "The {provider} sign-in has expired. The user must sign in with {provider} again. ({})",
                    e.message()
                ),
            ))
        }
        Err(RefreshError::ProviderNotConfigured) => {
            return Err((
                "PROVIDER_NOT_CONFIGURED".into(),
                format!("OAuth provider \"{provider}\" is not configured on this server"),
            ))
        }
        Err(e) => {
            return Err((
                "PROVIDER_UNAVAILABLE".into(),
                format!("Could not refresh the {provider} token. Try again. ({})", e.message()),
            ))
        }
    };
    // Sign-ins that only carried an id_token (native federated sign-in)
    // store no access token. There is nothing to hand out until the person
    // signs in through the provider's web flow.
    let Some(access_token) = account.access_token.filter(|t| !t.is_empty()) else {
        return Err((
            "REAUTH_REQUIRED".into(),
            format!(
                "The {provider} account has no access token. The user must sign in with {provider} again."
            ),
        ));
    };
    Ok(serde_json::json!({
        "accessToken": access_token,
        "expiresAt": account.access_token_expires_at,
        "scope": account.scope,
    }))
}

/// Resolve an access token this app's OIDC provider issued. `null` for
/// unknown, expired, revoked, and refresh tokens.
fn verify_oidc_token(token: &str) -> Result<serde_json::Value, (String, String)> {
    let Some(provider) = pylon_auth::oidc_provider::provider_or_init() else {
        return Err((
            "OIDC_NOT_CONFIGURED".into(),
            "ctx.auth.verifyOidcToken works only in an app that is an OIDC provider (set PYLON_OIDC_ISSUER)".into(),
        ));
    };
    match provider.lookup_access_token(token) {
        Ok(Some(record)) => Ok(serde_json::json!({
            "userId": record.user_id,
            "clientId": record.client_id,
            "scopes": record.scopes,
            "expiresAt": record.expires_at,
        })),
        Ok(None) => Ok(serde_json::Value::Null),
        Err(e) => {
            tracing::error!("[oidc] token store lookup failed: {e}");
            Err((
                "OIDC_STORE_UNAVAILABLE".into(),
                "Could not read the OIDC token store. Try again.".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(user: &str, provider: &str, account_id: &str, token: Option<&str>) -> Account {
        Account {
            id: format!("acc_{account_id}"),
            user_id: user.into(),
            provider_id: provider.into(),
            account_id: account_id.into(),
            access_token: token.map(String::from),
            refresh_token: Some("rt".into()),
            id_token: None,
            access_token_expires_at: Some(2_000_000_000),
            refresh_token_expires_at: None,
            scope: Some("openid offline_access".into()),
            password: None,
            avatar_url: None,
            external_orgs: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn store_with(accounts: &[Account]) -> AccountStore {
        let store = AccountStore::new();
        for a in accounts {
            store.upsert(a);
        }
        store
    }

    #[test]
    fn returns_the_refreshed_token_of_the_users_account() {
        let store = store_with(&[
            account("u1", "stack0", "sub-1", Some("at-1")),
            account("u2", "stack0", "sub-2", Some("at-2")),
        ]);
        let out = provider_token(&store, "stack0", "u1", |p, a| {
            assert_eq!((p, a), ("stack0", "sub-1"));
            Ok(account("u1", "stack0", "sub-1", Some("fresh")))
        })
        .unwrap();
        assert_eq!(out["accessToken"], "fresh");
        assert_eq!(out["expiresAt"], 2_000_000_000u64);
        assert_eq!(out["scope"], "openid offline_access");
    }

    #[test]
    fn null_when_the_user_has_no_account_with_that_provider() {
        let store = store_with(&[account("u1", "google", "g-1", Some("at"))]);
        let out = provider_token(&store, "stack0", "u1", |_, _| {
            panic!("no refresh without an account")
        })
        .unwrap();
        assert!(out.is_null());
        let out = provider_token(&store, "stack0", "u1", |_, _| {
            Err(RefreshError::AccountNotFound)
        })
        .unwrap();
        assert!(out.is_null());
    }

    #[test]
    fn a_refused_or_missing_refresh_requires_sign_in_again() {
        let store = store_with(&[account("u1", "stack0", "sub-1", Some("old"))]);
        for err in [
            RefreshError::NoRefreshToken,
            RefreshError::RefreshFailed(r#"HTTP 400: {"error":"invalid_grant"}"#.into()),
            RefreshError::RefreshFailed("HTTP 401: invalid_client".into()),
        ] {
            let (code, msg) =
                provider_token(&store, "stack0", "u1", |_, _| Err(err.clone())).unwrap_err();
            assert_eq!(code, "REAUTH_REQUIRED");
            assert!(!msg.contains("old"), "never leaks the stale token: {msg}");
        }
    }

    #[test]
    fn a_provider_outage_is_not_a_sign_in_problem() {
        let store = store_with(&[account("u1", "stack0", "sub-1", Some("old"))]);
        for err in [
            RefreshError::RefreshFailed("HTTP 503: down".into()),
            RefreshError::RefreshFailed("HTTP error: connection refused".into()),
        ] {
            let (code, _) =
                provider_token(&store, "stack0", "u1", |_, _| Err(err.clone())).unwrap_err();
            assert_eq!(code, "PROVIDER_UNAVAILABLE");
        }
        let (code, _) = provider_token(&store, "stack0", "u1", |_, _| {
            Err(RefreshError::ProviderNotConfigured)
        })
        .unwrap_err();
        assert_eq!(code, "PROVIDER_NOT_CONFIGURED");
    }

    #[test]
    fn an_account_without_an_access_token_requires_sign_in_again() {
        let store = store_with(&[account("u1", "stack0", "sub-1", Some(""))]);
        let (code, _) = provider_token(&store, "stack0", "u1", |_, _| {
            Ok(account("u1", "stack0", "sub-1", Some("")))
        })
        .unwrap_err();
        assert_eq!(code, "REAUTH_REQUIRED");
    }

    #[test]
    fn the_most_recent_of_two_linked_accounts_wins() {
        let mut older = account("u1", "stack0", "sub-old", Some("a"));
        older.updated_at = 10;
        let mut newer = account("u1", "stack0", "sub-new", Some("b"));
        newer.updated_at = 20;
        let store = store_with(&[older, newer]);
        let out = provider_token(&store, "stack0", "u1", |_, a| {
            assert_eq!(a, "sub-new");
            Ok(account("u1", "stack0", a, Some("b")))
        })
        .unwrap();
        assert_eq!(out["accessToken"], "b");
    }

    #[test]
    fn handle_op_reads_the_users_account_through_the_store() {
        // Expires far in the future: ensure_fresh_access_token returns it
        // without calling the provider.
        let store = store_with(&[account("u1", "stack0", "sub-1", Some("live"))]);
        let req = AuthOpMessage {
            call_id: "c".into(),
            op: "provider_token".into(),
            provider: Some("stack0".into()),
            user_id: None,
            token: None,
        };
        let out = handle_op(&store, &req, Some("u1")).unwrap();
        assert_eq!(out["accessToken"], "live");
        assert!(handle_op(&store, &req, Some("u2")).unwrap().is_null());
    }
}
