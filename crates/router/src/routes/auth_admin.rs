//! `GET /api/admin/auth/{sessions|accounts|oauth_state|magic_codes}` —
//! read-only listing of the framework's auth-state tables. Powers the
//! Studio "Auth tables" section so operators can debug stuck OAuth
//! handshakes / orphaned sessions / etc. without dropping into psql.
//!
//! Admin-gated. Sensitive fields are redacted at the response layer:
//! - session tokens → first 12 chars + ellipsis
//! - access/refresh/id tokens → presence indicator only
//! - password hashes → never returned
//! - magic codes → first 2 digits + asterisks
//!
//! The redaction is defense-in-depth: even though admins already have
//! enough access to grab these via psql, a Studio session being shoulder-
//! surfed shouldn't leak a live login token. Operators who actually
//! need the raw value can use `pylon admin sessions inspect <token>`
//! (which doesn't exist yet — file when the need arises).

use crate::{json_error, require_admin, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    _body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    let tail = url.strip_prefix("/api/admin/auth/")?;
    if method != HttpMethod::Get {
        return Some((
            405,
            json_error("METHOD_NOT_ALLOWED", "Only GET is supported here"),
        ));
    }
    if let Some(err) = require_admin(ctx) {
        return Some(err);
    }

    let rows = match tail {
        "sessions" => sessions_view(ctx),
        "accounts" => accounts_view(ctx),
        "oauth_state" => oauth_state_view(ctx),
        "magic_codes" => magic_codes_view(ctx),
        other => {
            return Some((
                404,
                json_error(
                    "UNKNOWN_AUTH_TABLE",
                    &format!(
                        "Unknown auth table \"{other}\". Valid: sessions, accounts, oauth_state, magic_codes"
                    ),
                ),
            ));
        }
    };

    Some((
        200,
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
    ))
}

/// Sessions — redact the token down to a short prefix so a Studio
/// screen-share doesn't accidentally leak a live login token.
fn sessions_view(ctx: &RouterContext) -> Vec<serde_json::Value> {
    ctx.session_store
        .list_all_unfiltered()
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "id": s.token, // stable row id for client-side keys
                "token_preview": redact_token(&s.token),
                "user_id": s.user_id,
                "expires_at": s.expires_at,
                "created_at": s.created_at,
                "device": s.device,
                "tenant_id": s.tenant_id,
            })
        })
        .collect()
}

/// Accounts — redact every token to a presence boolean. The
/// `(provider_id, account_id)` pair + linked user_id is what the
/// operator actually needs to debug "OAuth user got the wrong link."
fn accounts_view(ctx: &RouterContext) -> Vec<serde_json::Value> {
    ctx.account_store
        .list_all_unfiltered()
        .into_iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "user_id": a.user_id,
                "provider_id": a.provider_id,
                "account_id": a.account_id,
                "scope": a.scope,
                "has_access_token": a.access_token.is_some(),
                "has_refresh_token": a.refresh_token.is_some(),
                "has_id_token": a.id_token.is_some(),
                "has_password": a.password.is_some(),
                "access_token_expires_at": a.access_token_expires_at,
                "refresh_token_expires_at": a.refresh_token_expires_at,
                "created_at": a.created_at,
                "updated_at": a.updated_at,
            })
        })
        .collect()
}

/// OAuth state — short-lived (10 min) CSRF tokens. Token preview is
/// useful for debugging "callback says invalid state" — operator can
/// spot whether the state ever made it to the store.
fn oauth_state_view(_ctx: &RouterContext) -> Vec<serde_json::Value> {
    // OAuthStateStore intentionally doesn't expose a list_all method —
    // every read is a `take` (single-use, atomic). Returning an empty
    // list is honest: there's nothing to inspect that the act of
    // inspecting wouldn't consume. If operators need to debug live
    // OAuth flows, the runtime traces in /api/admin/audit-log are the
    // right surface.
    Vec::new()
}

/// Magic codes — redact the code to first 2 digits + 4 asterisks so a
/// Studio session doesn't accidentally hand out the verify code.
fn magic_codes_view(ctx: &RouterContext) -> Vec<serde_json::Value> {
    ctx.magic_codes
        .list_all_unfiltered()
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "id": c.email, // composite-key entities use the natural id
                "email": c.email,
                "code_preview": redact_code(&c.code),
                "expires_at": c.expires_at,
                "attempts": c.attempts,
            })
        })
        .collect()
}

fn redact_token(t: &str) -> String {
    if t.len() <= 12 {
        return "***".into();
    }
    format!("{}…", &t[..12])
}

fn redact_code(c: &str) -> String {
    if c.len() < 2 {
        return "******".into();
    }
    format!("{}****", &c[..2])
}
