//! Live-Postgres integration tests for the four auth-state backends:
//! [`PostgresSessionBackend`], [`PostgresOAuthBackend`],
//! [`PostgresMagicCodeBackend`], [`PostgresAccountBackend`]. Schema
//! aligned with better-auth so a future migration tool can map rows
//! across.
//!
//! Skipped unless `PYLON_TEST_PG_URL` is set. Run locally with the same
//! docker-compose recipe as `postgres_backend.rs`.

use pylon_auth::{
    Account, AccountBackend, MagicCode, MagicCodeBackend, OAuthStateBackend, Session,
    SessionBackend,
};
use pylon_runtime::{
    account_backend::PostgresAccountBackend, magic_code_backend::PostgresMagicCodeBackend,
    oauth_backend::PostgresOAuthBackend, session_backend::PostgresSessionBackend,
};

fn pg_url() -> Option<String> {
    std::env::var("PYLON_TEST_PG_URL").ok()
}

#[test]
fn session_backend_roundtrip() {
    let Some(url) = pg_url() else {
        return;
    };
    let b = PostgresSessionBackend::connect(&url).expect("connect");
    let s = Session::new("user_pg_session".into());
    b.save(&s);
    let all = b.load_all();
    assert!(all.iter().any(|x| x.token == s.token));
    b.remove(&s.token);
    let all = b.load_all();
    assert!(!all.iter().any(|x| x.token == s.token));
}

#[test]
fn oauth_state_backend_take_is_atomic_single_use() {
    let Some(url) = pg_url() else {
        return;
    };
    let b = PostgresOAuthBackend::connect(&url).expect("connect");
    b.put("tok_pg_oauth", "google", 9_999_999_999);
    assert_eq!(b.take("tok_pg_oauth", 0).as_deref(), Some("google"));
    // Second take returns None — DELETE … RETURNING is atomic so
    // concurrent callbacks for the same token can't both succeed.
    assert!(b.take("tok_pg_oauth", 0).is_none());
}

#[test]
fn magic_code_backend_put_get_bump_remove() {
    let Some(url) = pg_url() else {
        return;
    };
    let b = PostgresMagicCodeBackend::connect(&url).expect("connect");
    let mc = MagicCode {
        email: "mc_pg@example.com".into(),
        code: "654321".into(),
        expires_at: 9_999_999_999,
        attempts: 0,
    };
    b.put(&mc.email, &mc);
    let got = b.get(&mc.email).expect("present");
    assert_eq!(got.code, "654321");
    b.bump_attempts(&mc.email);
    assert_eq!(b.get(&mc.email).unwrap().attempts, 1);
    b.remove(&mc.email);
    assert!(b.get(&mc.email).is_none());
}

#[test]
fn account_backend_better_auth_schema_full_roundtrip() {
    // Validates the full better-auth-aligned column set: id, user_id,
    // provider_id, account_id, access/refresh/id tokens, expires-at
    // pair, scope, password (for the future credential provider),
    // created/updated_at. UPSERT on (provider_id, account_id) refreshes
    // the token bundle without losing the row id.
    let Some(url) = pg_url() else {
        return;
    };
    let b = PostgresAccountBackend::connect(&url).expect("connect");

    let now: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let initial = Account {
        id: "acct_pg_full_1".into(),
        user_id: "user_pg_acct".into(),
        provider_id: "google".into(),
        account_id: "google_sub_full_1".into(),
        access_token: Some("at_v1".into()),
        refresh_token: Some("rt_v1".into()),
        id_token: Some("id_v1".into()),
        access_token_expires_at: Some(now + 3600),
        refresh_token_expires_at: Some(now + 30 * 24 * 3600),
        scope: Some("email profile openid".into()),
        password: None,
        created_at: now,
        updated_at: now,
    };
    b.upsert(&initial);
    let got = b
        .find_by_provider("google", "google_sub_full_1")
        .expect("present");
    assert_eq!(got.user_id, "user_pg_acct");
    assert_eq!(got.access_token.as_deref(), Some("at_v1"));
    assert_eq!(got.scope.as_deref(), Some("email profile openid"));
    assert!(got.access_token_expires_at.is_some());
    assert!(got.refresh_token_expires_at.is_some());

    // Re-upsert with refreshed tokens — id should stay stable on the
    // existing row because of the (provider_id, account_id) UNIQUE
    // constraint; only token fields + updated_at should change.
    let refreshed = Account {
        access_token: Some("at_v2".into()),
        updated_at: now + 100,
        ..initial.clone()
    };
    b.upsert(&refreshed);
    let got = b.find_by_provider("google", "google_sub_full_1").unwrap();
    assert_eq!(got.access_token.as_deref(), Some("at_v2"));
    assert_eq!(got.updated_at, now + 100);
}

#[test]
fn account_backend_credential_provider_stores_password() {
    // The `password` column reserves space for email/password auth
    // (better-auth's `provider_id="credential"` rows). This test
    // exists to guarantee the column is wired end-to-end against PG —
    // without it, adding password auth later would silently lose the
    // hash and fall through to "user has no way to sign in."
    let Some(url) = pg_url() else {
        return;
    };
    let b = PostgresAccountBackend::connect(&url).expect("connect");
    let now = 42u64;
    let cred = Account {
        id: "acct_pg_cred".into(),
        user_id: "user_pg_cred".into(),
        provider_id: "credential".into(),
        account_id: "user_pg_cred".into(),
        access_token: None,
        refresh_token: None,
        id_token: None,
        access_token_expires_at: None,
        refresh_token_expires_at: None,
        scope: None,
        password: Some("argon2id$dummy_hash".into()),
        created_at: now,
        updated_at: now,
    };
    b.upsert(&cred);
    let got = b.find_by_provider("credential", "user_pg_cred").unwrap();
    assert_eq!(got.password.as_deref(), Some("argon2id$dummy_hash"));
    assert!(got.access_token.is_none());
}

#[test]
fn account_backend_find_for_user_lists_multi_provider() {
    let Some(url) = pg_url() else {
        return;
    };
    let b = PostgresAccountBackend::connect(&url).expect("connect");
    let now = 1u64;
    let user = "user_pg_multi";
    for (provider, sub) in [("google", "g_sub_multi"), ("github", "gh_sub_multi")] {
        b.upsert(&Account {
            id: format!("acct_pg_{provider}"),
            user_id: user.into(),
            provider_id: provider.into(),
            account_id: sub.into(),
            access_token: Some("at".into()),
            refresh_token: None,
            id_token: None,
            access_token_expires_at: None,
            refresh_token_expires_at: None,
            scope: None,
            password: None,
            created_at: now,
            updated_at: now,
        });
    }
    let mine = b.find_for_user(user);
    assert_eq!(mine.len(), 2);
    assert!(mine.iter().any(|a| a.provider_id == "google"));
    assert!(mine.iter().any(|a| a.provider_id == "github"));

    assert!(b.unlink("github", "gh_sub_multi"));
    assert_eq!(b.find_for_user(user).len(), 1);
}
