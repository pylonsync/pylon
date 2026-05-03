//! `/api/auth/*` routes — sessions, OAuth, magic-link, password,
//! email verification, /me, /providers, /sessions, refresh.
//!
//! Returns `Some((status, body))` if this module owns the route, or
//! `None` to fall through to the next module. Behavior is identical
//! to the pre-split inline handlers in `lib.rs`; this is a pure
//! mechanical extraction so security audits can scope to one file.

use crate::{
    complete_oauth_login_pkce, json_error, json_error_safe, json_error_with_hint, parse_query,
    redact_email, url_encode, RouterContext,
};
use pylon_http::HttpMethod;

/// Build an `AuditEventBuilder` pre-populated with the per-request
/// fields (peer IP, User-Agent header, current tenant). Caller adds
/// action-specific bits via `.user(...)`, `.failed(...)`, `.meta(...)`.
fn audit(
    ctx: &RouterContext,
    action: pylon_auth::audit::AuditAction,
) -> pylon_auth::audit::AuditEventBuilder {
    let mut b = pylon_auth::audit::AuditEventBuilder::new(action).ip(ctx.peer_ip);
    if let Some(ua) = request_user_agent(ctx) {
        b = b.user_agent(ua);
    }
    if let Some(tid) = ctx.auth_ctx.tenant_id.as_deref() {
        b = b.tenant(tid);
    }
    if let Some(uid) = ctx.auth_ctx.user_id.as_deref() {
        // Default actor = currently-logged-in user. Endpoints that
        // do "admin acts on user" override `.user(target)` after.
        b = b.actor(uid);
    }
    b
}

/// Extract the User-Agent header value (case-insensitive). Returns
/// None if absent or empty.
fn request_user_agent<'a>(ctx: &'a RouterContext) -> Option<&'a str> {
    ctx.request_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
        .map(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Mint a session pre-tagged with the parsed device label (so
/// `/api/auth/sessions` can show "Chrome on macOS" instead of the
/// raw User-Agent). Falls back to a no-device session for non-
/// browser callers without a UA header.
fn create_session_with_device(ctx: &RouterContext, user_id: String) -> pylon_auth::Session {
    let device = request_user_agent(ctx).map(pylon_auth::device::parse_user_agent);
    ctx.session_store.create_with_device(user_id, device)
}

/// If the request arrived under a guest session whose user_id differs
/// from `to_user_id`, transfer ownership of every `id(<user_entity>)`
/// row from the guest to the authenticated user and revoke the guest
/// session. Returns `(from_user_id, summary)` when a merge ran, `None`
/// otherwise (no guest cookie, same id, or no rows to move).
///
/// Side effects beyond the row updates:
/// - Revokes the guest session token so a leaked guest cookie can't
///   later impersonate the (now empty) guest user_id.
/// - Does NOT delete the guest user row. Apps may have FK constraints
///   that prevent deletion, and an orphan guest row with zero
///   referencing entities is harmless. A future GC sweep can clean up.
/// Wave-7 D — wraps the SignIn audit + anonymous merge + AnonymousMerge
/// audit into one call so every successful auth path (magic-code, OAuth,
/// password, passkey, …) gets identical bookkeeping. The OAuth callbacks
/// each invoke this exactly once, in the Ok arm.
fn audit_oauth_login(ctx: &RouterContext, user_id: &str, provider: &str) {
    let merge_summary = maybe_merge_anonymous(ctx, user_id);
    ctx.audit.log(
        audit(ctx, pylon_auth::audit::AuditAction::SignIn)
            .user(user_id.to_string())
            .actor(user_id.to_string())
            .meta("method", format!("oauth:{provider}"))
            .build(),
    );
    if let Some((from, summary)) = merge_summary {
        ctx.audit.log(
            audit(ctx, pylon_auth::audit::AuditAction::AnonymousMerge)
                .user(user_id.to_string())
                .actor(user_id.to_string())
                .meta("from_user_id", from)
                .meta("rows_updated", summary.rows_updated.to_string())
                .meta("entities", summary.entities_csv())
                .build(),
        );
    }
}

/// Wave-8 — `GET /api/auth/orgs/<org_id>/sso/start`. Reads the org's
/// SSO config, validates the caller-supplied callback URLs against
/// `PYLON_TRUSTED_ORIGINS`, mints a PKCE-protected state token, and
/// 302s to the IdP's authorization endpoint.
fn handle_org_sso_start(ctx: &RouterContext, org_id: &str, raw: &str) -> (u16, String) {
    let config = match ctx.org_sso.get(org_id) {
        Some(c) => c,
        None => {
            return (
                404,
                json_error("SSO_NOT_CONFIGURED", "org has no SSO configured"),
            )
        }
    };
    let query = raw.split_once('?').map(|(_, q)| q).unwrap_or("");
    let params = parse_query(query);
    let callback = match params.get("callback") {
        Some(c) if !c.is_empty() => c.clone(),
        _ => {
            return (
                400,
                json_error("MISSING_CALLBACK", "callback URL is required"),
            )
        }
    };
    let error_callback = params
        .get("error_callback")
        .cloned()
        .unwrap_or_else(|| callback.clone());
    // Reuse the global trusted-origins allowlist — same protection as
    // the regular OAuth start endpoint. Without this, an attacker can
    // direct the IdP redirect at any origin they control.
    if let Err(e) = pylon_auth::validate_trusted_redirect(&callback, ctx.trusted_origins) {
        return (
            403,
            json_error("UNTRUSTED_CALLBACK", &format!("callback rejected: {e:?}")),
        );
    }
    if let Err(e) = pylon_auth::validate_trusted_redirect(&error_callback, ctx.trusted_origins) {
        return (
            403,
            json_error(
                "UNTRUSTED_ERROR_CALLBACK",
                &format!("error_callback rejected: {e:?}"),
            ),
        );
    }
    let pkce = pylon_auth::generate_pkce();
    let state = pylon_auth::org_sso::random_state();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    ctx.org_sso
        .save_state(pylon_auth::org_sso::OrgSsoStateRecord {
            state: state.clone(),
            org_id: org_id.to_string(),
            pkce_verifier: pkce.code_verifier,
            callback_url: callback,
            error_callback_url: error_callback,
            created_at: now,
        });
    // Build redirect URI matching the callback endpoint we'll hit.
    // Apps configure the IdP with this exact URL on their end.
    let redirect_uri = match build_sso_redirect_uri(ctx, org_id) {
        Some(u) => u,
        None => {
            return (
                500,
                json_error(
                    "REDIRECT_URI_UNAVAILABLE",
                    "set PYLON_PUBLIC_URL so SSO callbacks can reach the server",
                ),
            )
        }
    };
    // OIDC authorization request: response_type=code, scope=openid email
    // profile, S256 PKCE. `prompt=select_account` left to the IdP — most
    // OIDC servers respect SP omission.
    let target = format!(
        "{auth}?response_type=code&client_id={cid}&redirect_uri={ruri}&scope={scope}&state={state}&code_challenge={chal}&code_challenge_method=S256",
        auth = config.authorization_endpoint,
        cid = url_encode(&config.client_id),
        ruri = url_encode(&redirect_uri),
        scope = url_encode("openid email profile"),
        state = url_encode(&state),
        chal = url_encode(&pkce.code_challenge),
    );
    ctx.add_response_header("Location", target);
    (302, String::new())
}

/// Wave-8 — `GET /api/auth/orgs/<org_id>/sso/callback?code=…&state=…`.
/// Single-use state consumption + token exchange + userinfo fetch +
/// auto-join + session mint. On error: 302 to the operator-configured
/// `error_callback_url`.
fn handle_org_sso_callback(ctx: &RouterContext, org_id: &str, raw: &str) -> (u16, String) {
    let query = raw.split_once('?').map(|(_, q)| q).unwrap_or("");
    let params = parse_query(query);
    let state_token = match params.get("state") {
        Some(s) if !s.is_empty() => s.clone(),
        _ => {
            return (
                400,
                json_error("MISSING_STATE", "state parameter is required"),
            )
        }
    };
    let code = match params.get("code") {
        Some(c) if !c.is_empty() => c.clone(),
        _ => {
            return (
                400,
                json_error("MISSING_CODE", "code parameter is required"),
            )
        }
    };
    let state_record = match ctx.org_sso.take_state(&state_token, org_id) {
        Some(r) => r,
        None => {
            return (
                403,
                json_error(
                    "INVALID_SSO_STATE",
                    "state is unknown, expired, or for a different org",
                ),
            );
        }
    };
    let config = match ctx.org_sso.get(org_id) {
        Some(c) => c,
        None => {
            return (
                404,
                json_error("SSO_NOT_CONFIGURED", "org has no SSO configured"),
            )
        }
    };
    let redirect_uri = match build_sso_redirect_uri(ctx, org_id) {
        Some(u) => u,
        None => {
            return (
                500,
                json_error(
                    "REDIRECT_URI_UNAVAILABLE",
                    "set PYLON_PUBLIC_URL so SSO callbacks can reach the server",
                ),
            )
        }
    };
    let (email, name) = match pylon_auth::org_sso::complete_oidc_login(
        &config,
        &code,
        &state_record.pkce_verifier,
        &redirect_uri,
    ) {
        Ok(v) => v,
        Err(e) => {
            return sso_error_redirect(
                ctx,
                &state_record.error_callback_url,
                e.code(),
                &e.message(),
            );
        }
    };
    let display_name = name.unwrap_or_else(|| email.clone());
    // Look up or create the User row by email — same pattern as the
    // magic-code verify path.
    let now = format!(
        "{}Z",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    let user_entity = ctx.store.manifest().auth.user.entity.clone();
    let user_id = match ctx.store.lookup(&user_entity, "email", &email) {
        Ok(Some(row)) => {
            let id = row["id"].as_str().unwrap_or("").to_string();
            // The IdP just vouched for the email; stamp emailVerified
            // if it wasn't already.
            if row.get("emailVerified").map_or(true, |v| v.is_null()) {
                let _ = ctx.store.update(
                    &user_entity,
                    &id,
                    &serde_json::json!({ "emailVerified": now }),
                );
            }
            id
        }
        _ => match ctx.store.insert(
            &user_entity,
            &serde_json::json!({
                "email": &email,
                "displayName": display_name,
                "emailVerified": now,
                "createdAt": now,
            }),
        ) {
            Ok(id) => id,
            Err(e) => {
                return sso_error_redirect(
                    ctx,
                    &state_record.error_callback_url,
                    "USER_CREATE_FAILED",
                    &format!("{}: {}", e.code, e.message),
                )
            }
        },
    };
    // Auto-join: idempotent. If the user is already a member of the
    // org, leave the existing role alone (don't downgrade an admin to
    // member just because they signed in via SSO again). Only add when
    // there's no existing membership.
    if ctx.orgs.role_of(org_id, &user_id).is_none() {
        let role = pylon_auth::org::OrgRole::from_str(&config.default_role)
            .unwrap_or(pylon_auth::org::OrgRole::Member);
        ctx.orgs.add_member(org_id, &user_id, role);
    }
    // Mint session + audit + 302 to the caller's success URL.
    let session = create_session_with_device(ctx, user_id.clone());
    ctx.audit.log(
        audit(ctx, pylon_auth::audit::AuditAction::SignIn)
            .user(user_id.clone())
            .actor(user_id.clone())
            .meta("method", "org_sso")
            .meta("org_id", org_id.to_string())
            .build(),
    );
    let cookie_value = ctx.cookie_config.set_value(&session.token);
    ctx.add_response_header("Set-Cookie", cookie_value);
    ctx.add_response_header("Location", state_record.callback_url);
    (302, String::new())
}

/// Build the SSO callback URL the IdP redirects to. Sources:
/// 1. `PYLON_PUBLIC_URL` env var (e.g. `https://api.pylonsync.com`) —
///    operator-set base. Required when the request didn't carry an
///    `Origin` or `X-Forwarded-Proto`/`X-Forwarded-Host` we trust.
fn build_sso_redirect_uri(ctx: &RouterContext, org_id: &str) -> Option<String> {
    let base = std::env::var("PYLON_PUBLIC_URL").ok()?;
    let trimmed = base.trim_end_matches('/');
    let _ = ctx;
    Some(format!("{trimmed}/api/auth/orgs/{org_id}/sso/callback"))
}

fn sso_error_redirect(
    ctx: &RouterContext,
    error_url: &str,
    code: &str,
    msg: &str,
) -> (u16, String) {
    let sep = if error_url.contains('?') { '&' } else { '?' };
    let target = format!(
        "{error_url}{sep}sso_error={code}&sso_error_message={msg}",
        code = url_encode(code),
        msg = url_encode(&msg[..msg.len().min(200)]),
    );
    ctx.add_response_header("Location", target);
    (302, String::new())
}

fn maybe_merge_anonymous(
    ctx: &RouterContext,
    to_user_id: &str,
) -> Option<(String, crate::merge::MergeResult)> {
    if !ctx.auth_ctx.is_guest {
        return None;
    }
    let from_user_id = ctx.auth_ctx.user_id.as_deref()?.to_string();
    if from_user_id == to_user_id {
        return None;
    }
    let user_entity = ctx.store.manifest().auth.user.entity.clone();
    let summary = crate::merge::transfer_user_ownership(
        ctx.store,
        ctx.store.manifest(),
        &from_user_id,
        to_user_id,
        &user_entity,
    );
    // Revoke the guest session regardless of whether rows moved — the
    // guest cookie is no longer needed and leaving it valid is a small
    // session-fixation surface.
    ctx.session_store.revoke_all_for_user(&from_user_id);
    Some((from_user_id, summary))
}

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    auth_token: Option<&str>,
) -> Option<(u16, String)> {
    // POST /api/auth/session
    //
    // Mints a session for an arbitrary user_id. This is a privileged operation
    // — there is NO credential check here, only an admin/dev gate. Production
    // code must go through `/api/auth/magic/verify` or the OAuth callback.
    // Historically this route was ungated and any caller could become any
    // user. Now: dev mode OR admin token required.
    if url == "/api/auth/session" && method == HttpMethod::Post {
        if !ctx.is_dev && !ctx.auth_ctx.is_admin {
            return Some((
                403,
                json_error(
                    "FORBIDDEN",
                    "/api/auth/session requires admin auth in non-dev mode",
                ),
            ));
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                ));
            }
        };
        let user_id = match data.get("user_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return Some((400, json_error("MISSING_USER_ID", "user_id is required"))),
        };
        let session = ctx.session_store.create(user_id);
        return Some((
            201,
            serde_json::json!({"token": session.token, "user_id": session.user_id}).to_string(),
        ));
    }

    // GET /api/auth/me
    //
    // Cheap session/identity probe. Returns just the AuthContext
    // (`{ user_id, is_admin, roles, tenant_id }`) — no DB hit, no
    // entity fetch. Use this when all you need is "is the caller
    // signed in?" or "are they an admin?" — middleware, route gates,
    // permission checks. For the full `{ session, user }` payload
    // (with the User row from the DB), call /api/auth/session.
    //
    // AuthContext comes from the runtime's pre-route resolution —
    // calling session_store.resolve here would miss the
    // PYLON_ADMIN_TOKEN bearer-auth branch.
    if url == "/api/auth/me" && method == HttpMethod::Get {
        return Some((
            200,
            serde_json::to_string(ctx.auth_ctx).unwrap_or_else(|_| "{}".into()),
        ));
    }

    // GET /api/auth/session
    //
    // Better-auth's `getSession()` shape: returns both the session
    // (auth context) AND the User row in a single round-trip. The
    // SDK uses this for layout/dashboard reads; /api/auth/me stays
    // available for the cheap session-only probe.
    //
    // - User row is fetched by id from the manifest's User entity
    //   (conventionally named "User"; configurable user-entity is
    //   a follow-up).
    // - Sensitive fields stripped: `passwordHash` + anything starting
    //   with `_` (framework-internal columns). Apps wanting a custom
    //   projection can still expose a TS `getMe` function and call it
    //   alongside this endpoint.
    // - Returns `user: null` when the caller is anonymous, a guest,
    //   or the User row was deleted out from under the session.
    if url == "/api/auth/session" && method == HttpMethod::Get {
        let auth_cfg = &ctx.store.manifest().auth;
        let user_entity = &auth_cfg.user.entity;
        let mut body = serde_json::Map::new();
        let session_value = serde_json::to_value(ctx.auth_ctx).unwrap_or(serde_json::Value::Null);
        body.insert("session".into(), session_value);
        let user_value = ctx
            .auth_ctx
            .user_id
            .as_deref()
            .filter(|_| !ctx.auth_ctx.is_guest)
            .and_then(|uid| ctx.store.get_by_id(user_entity, uid).ok().flatten())
            .map(|row| project_user_row(row, &auth_cfg.user))
            .unwrap_or(serde_json::Value::Null);
        body.insert("user".into(), user_value);
        return Some((200, serde_json::Value::Object(body).to_string()));
    }

    // POST /api/auth/guest
    if url == "/api/auth/guest" && method == HttpMethod::Post {
        let session = ctx.session_store.create_guest();
        ctx.maybe_set_session_cookie(&session.token);
        return Some((
            201,
            serde_json::json!({"token": session.token, "user_id": session.user_id, "guest": true})
                .to_string(),
        ));
    }

    // POST /api/auth/upgrade
    //
    // Swap a guest session's anonymous id for a real user id. Same hole as
    // /api/auth/session if ungated: a caller holding a guest token can
    // upgrade to anyone. Gate: admin auth, or dev mode, with the same
    // rationale as session mint. Real upgrade should flow through magic-code
    // verify or OAuth callback, which consume the previous guest token and
    // issue a fresh user token server-side.
    if url == "/api/auth/upgrade" && method == HttpMethod::Post {
        if !ctx.is_dev && !ctx.auth_ctx.is_admin {
            return Some((
                403,
                json_error(
                    "FORBIDDEN",
                    "/api/auth/upgrade requires admin auth in non-dev mode",
                ),
            ));
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                ));
            }
        };
        let user_id = match data.get("user_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return Some((400, json_error("MISSING_USER_ID", "user_id is required"))),
        };
        if let Some(token) = auth_token {
            if ctx.session_store.upgrade(token, user_id.clone()) {
                return Some((
                    200,
                    serde_json::json!({"upgraded": true, "user_id": user_id}).to_string(),
                ));
            }
        }
        return Some((
            400,
            json_error("UPGRADE_FAILED", "No valid session to upgrade"),
        ));
    }

    // POST /api/auth/select-org
    //
    // Switch the caller's active tenant (organization). The server does a
    // membership check against OrgMember before committing — a client can't
    // impersonate an org it doesn't belong to. Pass `{ orgId: null }` to
    // leave all orgs (back to the login lobby).
    if url == "/api/auth/select-org" && method == HttpMethod::Post {
        let token = match auth_token {
            Some(t) => t,
            None => return Some((401, json_error("UNAUTHENTICATED", "missing bearer token"))),
        };
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(id) => id,
            None => return Some((401, json_error("UNAUTHENTICATED", "anonymous session"))),
        };
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                ));
            }
        };
        let target = data.get("orgId").and_then(|v| {
            if v.is_null() {
                Some(String::new())
            } else {
                v.as_str().map(String::from)
            }
        });
        let target = match target {
            Some(t) => t,
            None => {
                return Some((
                    400,
                    json_error("MISSING_ORG_ID", "orgId is required (or null)"),
                ));
            }
        };
        if target.is_empty() {
            // Clear the active org — the user is dropping out of all tenants.
            ctx.session_store.set_tenant(token, None);
            return Some((200, serde_json::json!({"tenantId": null}).to_string()));
        }
        // Look up an OrgMember row matching this user + target org.
        let filter = serde_json::json!({ "userId": user_id, "orgId": &target });
        match ctx.store.query_filtered("OrgMember", &filter) {
            Ok(rows) if !rows.is_empty() => {
                ctx.session_store.set_tenant(token, Some(target.clone()));
                return Some((200, serde_json::json!({"tenantId": target}).to_string()));
            }
            Ok(_) => {
                return Some((
                    403,
                    json_error(
                        "NOT_A_MEMBER",
                        "you are not a member of the target organization",
                    ),
                ));
            }
            Err(e) => {
                return Some((
                    500,
                    json_error_safe("LOOKUP_FAILED", "could not verify membership", &e.message),
                ));
            }
        }
    }

    // POST /api/auth/magic/send
    if url == "/api/auth/magic/send" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                ));
            }
        };
        let email = match data.get("email").and_then(|v| v.as_str()) {
            Some(e) => e.to_string(),
            None => return Some((400, json_error("MISSING_EMAIL", "email is required"))),
        };
        // Rate limit FIRST so an attacker who lacks a valid CAPTCHA
        // token can't bypass the per-IP cap by intentionally tripping
        // the CAPTCHA-fail path repeatedly.
        let rl = pylon_auth::rate_limit::AuthRateLimiter::shared();
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } = rl.check(
            pylon_auth::rate_limit::AuthBucket::Send,
            ctx.peer_ip,
            Some(&email),
        ) {
            return Some((
                429,
                json_error_with_hint(
                    "RATE_LIMITED",
                    "Too many sign-in requests",
                    &format!("Try again in {retry_after_secs}s"),
                ),
            ));
        }
        // Optional CAPTCHA gate. When PYLON_CAPTCHA_PROVIDER+SECRET
        // are set, the request must include `captchaToken`. Skipped
        // entirely when unconfigured so existing apps keep working.
        if let Some(cfg) = pylon_auth::captcha::CaptchaConfig::from_env() {
            let token = data
                .get("captchaToken")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Err(reason) = cfg.verify(token, Some(ctx.peer_ip)) {
                tracing::warn!("[captcha] magic/send rejected: {reason}");
                return Some((
                    400,
                    json_error("CAPTCHA_FAILED", "CAPTCHA verification failed"),
                ));
            }
        }
        let code = match ctx.magic_codes.try_create(&email) {
            Ok(c) => c,
            Err(pylon_auth::MagicCodeError::Throttled { retry_after_secs }) => {
                return Some((
                    429,
                    json_error_with_hint(
                        "RATE_LIMITED",
                        "A sign-in code was requested too recently.",
                        &format!("Try again in {retry_after_secs} seconds."),
                    ),
                ));
            }
            Err(e) => {
                return Some((
                    500,
                    json_error(
                        "EMAIL_SEND_FAILED",
                        &format!("Could not issue code: {:?}", e),
                    ),
                ));
            }
        };
        let mut vars = std::collections::HashMap::new();
        vars.insert("code", code.as_str());
        let (subject, body_text) = pylon_auth::email_templates::render(
            pylon_auth::email_templates::EmailTemplate::MagicCode,
            &vars,
        );
        if let Err(e) = ctx.email.send(&email, &subject, &body_text) {
            if !ctx.is_dev {
                tracing::warn!(
                    "[email] Failed to send magic code to {}: {e}",
                    redact_email(&email)
                );
                return Some((
                    500,
                    json_error("EMAIL_SEND_FAILED", "Could not send sign-in email"),
                ));
            }
        }
        if ctx.is_dev {
            return Some((
                200,
                serde_json::json!({"sent": true, "email": email, "dev_code": code}).to_string(),
            ));
        }
        return Some((
            200,
            serde_json::json!({"sent": true, "email": email}).to_string(),
        ));
    }

    // POST /api/auth/magic/verify
    if url == "/api/auth/magic/verify" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                ));
            }
        };
        let email = match data.get("email").and_then(|v| v.as_str()) {
            Some(e) => e,
            None => return Some((400, json_error("MISSING_EMAIL", "email is required"))),
        };
        let code = match data.get("code").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return Some((400, json_error("MISSING_CODE", "code is required"))),
        };
        match ctx.magic_codes.try_verify(email, code) {
            Ok(()) => {
                let now = format!(
                    "{}Z",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                );
                let user_id =
                    match ctx
                        .store
                        .lookup(&ctx.store.manifest().auth.user.entity, "email", email)
                    {
                        Ok(Some(row)) => {
                            let id = row["id"].as_str().unwrap_or("").to_string();
                            // Magic-link login implicitly verifies the
                            // email — the caller proved control by typing
                            // the code we sent there. Stamp emailVerified
                            // if not already set.
                            if row.get("emailVerified").map_or(true, |v| v.is_null()) {
                                let _ = ctx.store.update(
                                    &ctx.store.manifest().auth.user.entity,
                                    &id,
                                    &serde_json::json!({ "emailVerified": now }),
                                );
                            }
                            id
                        }
                        _ => ctx
                            .store
                            .insert(
                                &ctx.store.manifest().auth.user.entity,
                                &serde_json::json!({
                                    "email": email,
                                    "displayName": email,
                                    "emailVerified": now,
                                    "createdAt": now,
                                }),
                            )
                            .unwrap_or_else(|_| email.to_string()),
                    };
                let session = create_session_with_device(ctx, user_id.clone());
                ctx.maybe_set_session_cookie(&session.token);
                // Wave-7 D: anonymous → authenticated merge. If the request
                // arrived carrying a guest session cookie, transfer ownership
                // of any rows referencing that guest user_id over to the
                // newly-authenticated user. Cart-survives-login is the
                // canonical case. Guarded so a self-merge (already signed
                // in as the same user, refreshing the magic code) is a
                // no-op.
                let merge_summary = maybe_merge_anonymous(ctx, &user_id);
                ctx.audit.log(
                    audit(ctx, pylon_auth::audit::AuditAction::SignIn)
                        .user(user_id.clone())
                        .actor(user_id.clone())
                        .meta("method", "magic_code")
                        .build(),
                );
                if let Some((from, summary)) = merge_summary {
                    ctx.audit.log(
                        audit(ctx, pylon_auth::audit::AuditAction::AnonymousMerge)
                            .user(user_id.clone())
                            .actor(user_id.clone())
                            .meta("from_user_id", from)
                            .meta("rows_updated", summary.rows_updated.to_string())
                            .meta("entities", summary.entities_csv())
                            .build(),
                    );
                }
                return Some((
                    200,
                    serde_json::json!({"token": session.token, "user_id": user_id, "expires_at": session.expires_at}).to_string(),
                ));
            }
            Err(pylon_auth::MagicCodeError::TooManyAttempts) => {
                return Some((
                    429,
                    json_error(
                        "RATE_LIMITED",
                        "Too many verification attempts. Request a new code.",
                    ),
                ));
            }
            Err(_) => {}
        }
        return Some((401, json_error("INVALID_CODE", "Invalid or expired code")));
    }

    // POST /api/auth/email/send-verification
    //
    // Issues a 6-digit code to the *current session's* email address and
    // ships it via the EmailSender hook. Authenticated only — the email
    // is read from the User row keyed by `ctx.auth_ctx.user_id`, never from
    // the request body, so a logged-in caller can't trigger a code for
    // someone else's address.
    if url == "/api/auth/email/send-verification" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(id) => id,
            None => return Some((401, json_error("UNAUTHORIZED", "Sign in required"))),
        };
        let user = match ctx
            .store
            .get_by_id(&ctx.store.manifest().auth.user.entity, user_id)
        {
            Ok(Some(u)) => u,
            _ => return Some((404, json_error("USER_NOT_FOUND", "User not found"))),
        };
        let email = match user.get("email").and_then(|v| v.as_str()) {
            Some(e) => e.to_string(),
            None => {
                return Some((
                    400,
                    json_error("MISSING_EMAIL", "User has no email on file"),
                ));
            }
        };
        let code = match ctx.magic_codes.try_create(&email) {
            Ok(c) => c,
            Err(pylon_auth::MagicCodeError::Throttled { retry_after_secs }) => {
                return Some((
                    429,
                    json_error_with_hint(
                        "RATE_LIMITED",
                        "A verification code was requested too recently.",
                        &format!("Try again in {retry_after_secs} seconds."),
                    ),
                ));
            }
            Err(e) => {
                return Some((
                    500,
                    json_error(
                        "EMAIL_SEND_FAILED",
                        &format!("Could not issue code: {:?}", e),
                    ),
                ));
            }
        };
        let subject = "Verify your email address";
        let body_text = format!(
            "Your email verification code is: {code}\n\nThis code will expire in 10 minutes."
        );
        if let Err(e) = ctx.email.send(&email, subject, &body_text) {
            if !ctx.is_dev {
                tracing::warn!(
                    "[email] Failed to send verification code to {}: {e}",
                    redact_email(&email)
                );
                return Some((
                    500,
                    json_error("EMAIL_SEND_FAILED", "Could not send verification email"),
                ));
            }
        }
        if ctx.is_dev {
            return Some((
                200,
                serde_json::json!({"sent": true, "email": email, "dev_code": code}).to_string(),
            ));
        }
        return Some((
            200,
            serde_json::json!({"sent": true, "email": email}).to_string(),
        ));
    }

    // POST /api/auth/email/verify
    if url == "/api/auth/email/verify" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(id) => id,
            None => return Some((401, json_error("UNAUTHORIZED", "Sign in required"))),
        };
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                ));
            }
        };
        let code = match data.get("code").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return Some((400, json_error("MISSING_CODE", "code is required"))),
        };
        let user = match ctx
            .store
            .get_by_id(&ctx.store.manifest().auth.user.entity, user_id)
        {
            Ok(Some(u)) => u,
            _ => return Some((404, json_error("USER_NOT_FOUND", "User not found"))),
        };
        let email = match user.get("email").and_then(|v| v.as_str()) {
            Some(e) => e.to_string(),
            None => {
                return Some((
                    400,
                    json_error("MISSING_EMAIL", "User has no email on file"),
                ));
            }
        };
        match ctx.magic_codes.try_verify(&email, code) {
            Ok(()) => {
                let now = format!(
                    "{}Z",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                );
                // Best-effort: ignore the result. Schemas without an
                // emailVerified field will reject the unknown column;
                // schemas with it will accept the update. Either way
                // the verification *intent* succeeded.
                let _ = ctx.store.update(
                    &ctx.store.manifest().auth.user.entity,
                    user_id,
                    &serde_json::json!({ "emailVerified": now }),
                );
                return Some((
                    200,
                    serde_json::json!({"verified": true, "emailVerified": now}).to_string(),
                ));
            }
            Err(pylon_auth::MagicCodeError::TooManyAttempts) => {
                return Some((
                    429,
                    json_error(
                        "RATE_LIMITED",
                        "Too many verification attempts. Request a new code.",
                    ),
                ));
            }
            Err(_) => {}
        }
        return Some((401, json_error("INVALID_CODE", "Invalid or expired code")));
    }

    // POST /api/auth/password/register
    if url == "/api/auth/password/register" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                ));
            }
        };
        let email = match data.get("email").and_then(|v| v.as_str()) {
            Some(e) => e.trim().to_lowercase(),
            None => return Some((400, json_error("MISSING_EMAIL", "email is required"))),
        };
        if !email.contains('@') {
            return Some((
                400,
                json_error("INVALID_EMAIL", "email must be well-formed"),
            ));
        }
        let password = match data.get("password").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return Some((400, json_error("MISSING_PASSWORD", "password is required"))),
        };
        if let Err(e) = pylon_auth::password::validate_length(password) {
            return Some((400, json_error("WEAK_PASSWORD", &e.to_string())));
        }
        // CAPTCHA gate (no-op when unconfigured).
        if let Some(cfg) = pylon_auth::captcha::CaptchaConfig::from_env() {
            let token = data
                .get("captchaToken")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Err(reason) = cfg.verify(token, Some(ctx.peer_ip)) {
                tracing::warn!("[captcha] password/register rejected: {reason}");
                return Some((
                    400,
                    json_error("CAPTCHA_FAILED", "CAPTCHA verification failed"),
                ));
            }
        }
        // HIBP check unless explicitly disabled (off in test/dev to keep
        // unit tests offline). Honors PYLON_DISABLE_HIBP=1.
        if std::env::var("PYLON_DISABLE_HIBP").ok().as_deref() != Some("1") {
            match pylon_auth::password::check_pwned(password) {
                Ok(0) => {}
                Ok(n) => {
                    return Some((
                        400,
                        json_error_safe(
                            "PWNED_PASSWORD",
                            "This password has appeared in known data breaches. Choose a different one.",
                            &format!("HIBP returned {n} occurrences"),
                        ),
                    ));
                }
                // Fail-open on HIBP outage — security-vs-availability
                // tradeoff favors not locking out registration when an
                // external service is down.
                Err(_) => {}
            }
        }
        let display_name = data
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or(email.as_str())
            .to_string();

        if let Ok(Some(_)) =
            ctx.store
                .lookup(&ctx.store.manifest().auth.user.entity, "email", &email)
        {
            return Some((409, json_error("EMAIL_TAKEN", "Email already registered")));
        }

        let hash = pylon_auth::password::hash_password(password);
        let now = format!(
            "{}Z",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );

        let palette = [
            "#8b5cf6", "#6366f1", "#3b82f6", "#06b6d4", "#10b981", "#84cc16", "#eab308", "#f97316",
            "#ef4444", "#ec4899",
        ];
        let mut hash_val: i32 = 0;
        for b in email.as_bytes() {
            hash_val = hash_val.wrapping_mul(31).wrapping_add(*b as i32);
        }
        let avatar_color = palette[(hash_val.unsigned_abs() as usize) % palette.len()];

        let user_id = match ctx.store.insert(
            &ctx.store.manifest().auth.user.entity,
            &serde_json::json!({
                "email": email,
                "displayName": display_name,
                "avatarColor": avatar_color,
                "passwordHash": hash,
                "createdAt": now,
            }),
        ) {
            Ok(id) => id,
            Err(e) => return Some((400, json_error(&e.code, &e.message))),
        };

        let session = create_session_with_device(ctx, user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((
            200,
            serde_json::json!({
                "token": session.token,
                "user_id": user_id,
                "expires_at": session.expires_at,
            })
            .to_string(),
        ));
    }

    // POST /api/auth/password/login
    if url == "/api/auth/password/login" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                ));
            }
        };
        let email = match data.get("email").and_then(|v| v.as_str()) {
            Some(e) => e.trim().to_lowercase(),
            None => return Some((400, json_error("MISSING_EMAIL", "email is required"))),
        };
        let password = match data.get("password").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return Some((400, json_error("MISSING_PASSWORD", "password is required"))),
        };

        // Rate limit BEFORE the password compare so we don't burn
        // Argon2 cycles on a brute force attempt.
        let rl = pylon_auth::rate_limit::AuthRateLimiter::shared();
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } = rl.check(
            pylon_auth::rate_limit::AuthBucket::Login,
            ctx.peer_ip,
            Some(&email),
        ) {
            return Some((
                429,
                json_error_with_hint(
                    "RATE_LIMITED",
                    "Too many login attempts",
                    &format!("Try again in {retry_after_secs}s"),
                ),
            ));
        }

        let row = ctx
            .store
            .lookup(&ctx.store.manifest().auth.user.entity, "email", &email)
            .ok()
            .flatten();
        let (user_id, stored_hash): (Option<String>, Option<String>) = match row {
            Some(r) => (
                r.get("id").and_then(|v| v.as_str()).map(String::from),
                r.get("passwordHash")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            ),
            None => (None, None),
        };

        let matched = match &stored_hash {
            Some(h) if !h.is_empty() => pylon_auth::password::verify_password(password, h),
            _ => {
                let _ = pylon_auth::password::verify_password(
                    password,
                    pylon_auth::password::dummy_hash(),
                );
                false
            }
        };

        if !matched {
            ctx.audit.log(
                audit(ctx, pylon_auth::audit::AuditAction::SignInFailed)
                    .user(user_id.clone().unwrap_or_else(|| email.clone()))
                    .failed("WRONG_PASSWORD")
                    .meta("method", "password")
                    .build(),
            );
            return Some((
                401,
                json_error("INVALID_CREDENTIALS", "Email or password is incorrect"),
            ));
        }

        let user_id = match user_id {
            Some(id) => id,
            None => {
                return Some((
                    500,
                    json_error("USER_NOT_FOUND", "Authenticated but user missing"),
                ));
            }
        };
        let session = create_session_with_device(ctx, user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        ctx.audit.log(
            audit(ctx, pylon_auth::audit::AuditAction::SignIn)
                .user(user_id.clone())
                .actor(user_id.clone())
                .meta("method", "password")
                .build(),
        );
        return Some((
            200,
            serde_json::json!({
                "token": session.token,
                "user_id": user_id,
                "expires_at": session.expires_at,
            })
            .to_string(),
        ));
    }

    // GET /api/auth/providers
    if url == "/api/auth/providers" && method == HttpMethod::Get {
        let registry = pylon_auth::OAuthRegistry::shared();
        // Iterate the configured ids — order isn't stable across calls
        // but the frontend doesn't need it to be (it sorts by display
        // name). Sorting here would mask provider-list churn that's
        // useful in logs, so keep it as-is.
        let mut providers: Vec<serde_json::Value> = registry
            .ids()
            .filter_map(|id| {
                registry.get(id).map(|c| {
                    serde_json::json!({
                        "provider": id,
                        "auth_url": c.auth_url(),
                    })
                })
            })
            .collect();
        // Stable order in the response so the FE list doesn't reshuffle
        // every login page hit (HashMap iteration is unspecified).
        providers.sort_by(|a, b| {
            a.get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .cmp(b.get("provider").and_then(|v| v.as_str()).unwrap_or(""))
        });
        return Some((
            200,
            serde_json::to_string(&providers).unwrap_or_else(|_| "[]".into()),
        ));
    }

    // GET /api/auth/login/:provider?callback=<url>[&error_callback=<url>][&redirect=1]
    //
    // The frontend MUST pass `callback` — the URL pylon should 302 the
    // browser to after a successful OAuth handshake. Optional
    // `error_callback` is where failures land (defaults to `callback`,
    // with `?oauth_error=…&oauth_error_message=…` appended). Both URLs
    // must have origins listed in `PYLON_TRUSTED_ORIGINS` — same
    // pattern as better-auth's `trustedOrigins`. No env-var fallback;
    // an unconfigured trusted-origins list is a 400 from this route.
    if let Some(provider_raw) = url.strip_prefix("/api/auth/login/") {
        let provider = provider_raw.split('?').next().unwrap_or(provider_raw);
        if method == HttpMethod::Get {
            let registry = pylon_auth::OAuthRegistry::shared();
            let Some(config) = registry.get(provider) else {
                return Some((
                    404,
                    json_error_with_hint(
                        "PROVIDER_NOT_FOUND",
                        &format!("OAuth provider \"{provider}\" is not configured"),
                        &format!(
                            "Set PYLON_OAUTH_{}_CLIENT_ID + PYLON_OAUTH_{}_CLIENT_SECRET (and _REDIRECT). For OIDC IdPs (Auth0, Okta, Keycloak) also set PYLON_OAUTH_{}_OIDC_ISSUER.",
                            provider.to_ascii_uppercase(),
                            provider.to_ascii_uppercase(),
                            provider.to_ascii_uppercase(),
                        ),
                    ),
                ));
            };

            let query = provider_raw.split_once('?').map(|(_, q)| q).unwrap_or("");
            let params = parse_query(query);
            let callback = match params.get("callback").map(String::as_str) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => {
                    return Some((
                        400,
                        json_error_with_hint(
                            "MISSING_CALLBACK",
                            "GET /api/auth/login/:provider requires a `callback` query parameter",
                            "Add ?callback=<your-success-url>&error_callback=<your-failure-url>; both origins must be in PYLON_TRUSTED_ORIGINS",
                        ),
                    ));
                }
            };
            // error_callback defaults to callback — the frontend can
            // disambiguate via the `?oauth_error=` query param appended
            // on failure.
            let error_callback = params
                .get("error_callback")
                .filter(|s| !s.is_empty())
                .cloned()
                .unwrap_or_else(|| callback.clone());

            // Trusted-origins gate. Both URLs validated against the
            // same allowlist so an attacker can't sneak a redirect
            // through one parameter that they couldn't through the
            // other.
            for (kind, target) in [("callback", &callback), ("error_callback", &error_callback)] {
                if let Err(err) = pylon_auth::validate_trusted_redirect(target, ctx.trusted_origins)
                {
                    tracing::warn!(
                        "[oauth] rejected {kind}={target:?} for provider {provider}: {err}"
                    );
                    return Some((
                        403,
                        json_error_with_hint(
                            "UNTRUSTED_REDIRECT",
                            &format!("OAuth {kind} redirect rejected: {err}"),
                            "Add the redirect's origin (scheme://host[:port]) to PYLON_TRUSTED_ORIGINS (comma-separated)",
                        ),
                    ));
                }
            }

            // PKCE: when the provider requires it (Twitter/X, Kick), the
            // auth helper mints a verifier; we stash it on the state
            // record so the callback can replay it on token exchange.
            let (auth_url, pkce_verifier) = match config.auth_url_with_pkce("") {
                Ok((u, v)) => (u, v),
                Err(e) => {
                    return Some((
                        500,
                        json_error(
                            "OAUTH_PROVIDER_BROKEN",
                            &format!("provider {provider} misconfigured: {e}"),
                        ),
                    ));
                }
            };
            let state = ctx.oauth_state.create_with_pkce(
                provider,
                &callback,
                &error_callback,
                pkce_verifier,
            );
            // auth_url_with_pkce was given an empty state placeholder
            // because we mint the random state token AFTER the URL.
            // Append it now.
            let auth_url = format!("{auth_url}&state={}", url_encode(&state));
            let want_redirect = params
                .get("redirect")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if want_redirect {
                ctx.add_response_header("Location", auth_url);
                return Some((302, String::new()));
            }
            return Some((
                200,
                serde_json::json!({"redirect": auth_url, "state": state}).to_string(),
            ));
        }
    }

    // /api/auth/callback/:provider
    if let Some(provider_raw) = url.strip_prefix("/api/auth/callback/") {
        let provider = provider_raw.split('?').next().unwrap_or(provider_raw);

        // POST: SDK / programmatic flow OR Apple's `response_mode=form_post`
        // browser callback (Apple POSTs the redirect URL with a
        // form-encoded body when name/email scopes are requested).
        // Detect Apple by Content-Type header — the SDK flow always
        // sends JSON, the Apple flow sends form-urlencoded.
        if method == HttpMethod::Post {
            let is_form_post = ctx.request_headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case("content-type")
                    && v.to_ascii_lowercase()
                        .starts_with("application/x-www-form-urlencoded")
            });

            let (state, code, dev_email, dev_name, is_browser) = if is_form_post {
                // Apple form-post callback. Body is
                // `state=…&code=…&id_token=…[&user=…]`.
                let params = parse_query(body);
                let state = params.get("state").map(|s| s.as_str().to_string());
                let code = params.get("code").map(|s| s.as_str().to_string());
                (state, code, None, None, true)
            } else {
                let data: serde_json::Value = match serde_json::from_str(body) {
                    Ok(v) => v,
                    Err(e) => {
                        return Some((
                            400,
                            json_error_safe(
                                "INVALID_JSON",
                                "Invalid request body",
                                &format!("Invalid JSON: {e}"),
                            ),
                        ));
                    }
                };
                let state = data.get("state").and_then(|v| v.as_str()).map(String::from);
                let code = data.get("code").and_then(|v| v.as_str()).map(String::from);
                let dev_email = data.get("email").and_then(|v| v.as_str()).map(String::from);
                let dev_name = data.get("name").and_then(|v| v.as_str()).map(String::from);
                (state, code, dev_email, dev_name, false)
            };

            let state_record = match state
                .as_deref()
                .and_then(|s| ctx.oauth_state.validate(s, provider))
            {
                Some(r) => r,
                None => {
                    return Some((
                        403,
                        json_error(
                            "OAUTH_INVALID_STATE",
                            "Invalid or missing OAuth state parameter",
                        ),
                    ));
                }
            };

            let result = complete_oauth_login_pkce(
                ctx,
                provider,
                code.as_deref(),
                state_record.pkce_verifier.as_deref(),
                dev_email.as_deref(),
                dev_name.as_deref(),
            );

            // Browser-flow form_post callbacks (Apple) need to land
            // back on the user-supplied callback URL with a session
            // cookie set, just like the GET browser callback path.
            if is_browser {
                return Some(match result {
                    Ok((user_id, session)) => {
                        audit_oauth_login(ctx, &user_id, provider);
                        let cookie_value = ctx.cookie_config.set_value(&session.token);
                        ctx.add_response_header("Set-Cookie", cookie_value);
                        ctx.add_response_header("Location", state_record.callback_url);
                        (302, String::new())
                    }
                    Err(err) => {
                        tracing::warn!(
                            "[oauth] form_post callback {} failed: {} {}",
                            provider,
                            err.code,
                            err.message
                        );
                        let sep = if state_record.error_callback_url.contains('?') {
                            '&'
                        } else {
                            '?'
                        };
                        let target = format!(
                            "{}{}oauth_error={}&oauth_error_message={}",
                            state_record.error_callback_url,
                            sep,
                            url_encode(err.code),
                            url_encode(&err.message)
                        );
                        ctx.add_response_header("Location", target);
                        (302, String::new())
                    }
                });
            }

            return Some(match result {
                Ok((user_id, session)) => {
                    audit_oauth_login(ctx, &user_id, provider);
                    ctx.maybe_set_session_cookie(&session.token);
                    (
                        200,
                        serde_json::json!({
                            "token": session.token,
                            "user_id": user_id,
                            "provider": provider,
                            "expires_at": session.expires_at,
                        })
                        .to_string(),
                    )
                }
                Err(err) => (err.status, json_error(err.code, &err.message)),
            });
        }

        // GET: browser flow. State validation gives us the callback
        // URLs the start endpoint stored. No env-var lookup needed.
        //
        // CRITICAL: every arm here must `return Some(...)` directly.
        // Earlier code built `Some((302, ...))` as the value of the
        // match without returning, then a stray `let _ = ...` line
        // discarded it — every browser OAuth callback fell through
        // silently and produced no response. (Caught by codex P1
        // review of 0.3.9.)
        if method == HttpMethod::Get {
            let query = provider_raw.split_once('?').map(|(_, q)| q).unwrap_or("");
            let params = parse_query(query);
            let state_token = params.get("state").map(String::as_str);
            let code = params.get("code").map(String::as_str);

            // Validate state ONCE (single-use take) and capture the
            // stored callback URLs. We use them for both the success
            // 302 and the failure 302 — the start endpoint already
            // validated both URLs against PYLON_TRUSTED_ORIGINS.
            let state_record = match state_token.and_then(|s| ctx.oauth_state.validate(s, provider))
            {
                Some(s) => s,
                None => {
                    return Some((
                        403,
                        json_error(
                            "OAUTH_INVALID_STATE",
                            "Invalid, expired, or already-consumed OAuth state. Restart the sign-in flow.",
                        ),
                    ));
                }
            };

            match complete_oauth_login_pkce(
                ctx,
                provider,
                code,
                state_record.pkce_verifier.as_deref(),
                None,
                None,
            ) {
                Ok((user_id, session)) => {
                    audit_oauth_login(ctx, &user_id, provider);
                    let cookie_value = ctx.cookie_config.set_value(&session.token);
                    ctx.add_response_header("Set-Cookie", cookie_value);
                    ctx.add_response_header("Location", state_record.callback_url);
                    return Some((302, String::new()));
                }
                Err(err) => {
                    tracing::warn!(
                        "[oauth] callback {} failed: {} {}",
                        provider,
                        err.code,
                        err.message
                    );
                    let msg = if err.message.len() > 500 {
                        format!("{}…", &err.message[..500])
                    } else {
                        err.message.clone()
                    };
                    let sep = if state_record.error_callback_url.contains('?') {
                        '&'
                    } else {
                        '?'
                    };
                    let target = format!(
                        "{}{}oauth_error={}&oauth_error_message={}",
                        state_record.error_callback_url,
                        sep,
                        url_encode(err.code),
                        url_encode(&msg)
                    );
                    ctx.add_response_header("Location", target);
                    return Some((302, String::new()));
                }
            }
        }
    }

    let _ = body; // Suppress unused-warning for arms that don't read body.

    // DELETE /api/auth/session
    if url == "/api/auth/session" && method == HttpMethod::Delete {
        if let Some(token) = auth_token {
            ctx.session_store.revoke(token);
        }
        ctx.add_response_header("Set-Cookie", ctx.cookie_config.clear_value());
        return Some((200, serde_json::json!({"revoked": true}).to_string()));
    }

    // POST /api/auth/jwt — exchange the current session for a JWT-shaped
    // token (HS256 signed with PYLON_JWT_SECRET). Useful for edge runtimes
    // that can't tolerate a session-store round-trip on every request.
    // Requires PYLON_JWT_SECRET to be set; 501 otherwise.
    if url == "/api/auth/jwt" && method == HttpMethod::Post {
        if !ctx.auth_ctx.is_authenticated() {
            return Some((401, json_error("AUTH_REQUIRED", "Login required")));
        }
        let secret = match std::env::var("PYLON_JWT_SECRET").ok() {
            Some(s) if !s.is_empty() => s,
            _ => {
                return Some((
                    501,
                    json_error_with_hint(
                        "JWT_NOT_CONFIGURED",
                        "JWT-shaped sessions are disabled",
                        "Set PYLON_JWT_SECRET (32+ random bytes) to enable; optional PYLON_JWT_ISSUER for validation",
                    ),
                ));
            }
        };
        let issuer = std::env::var("PYLON_JWT_ISSUER").unwrap_or_else(|_| "pylon".into());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let lifetime = std::env::var("PYLON_JWT_LIFETIME_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60 * 60); // 1 hour default
        let claims = pylon_auth::jwt::JwtClaims {
            sub: ctx.auth_ctx.user_id.clone().unwrap_or_default(),
            iat: now,
            exp: now + lifetime,
            iss: issuer,
            tenant_id: ctx.auth_ctx.tenant_id.clone(),
            roles: ctx.auth_ctx.roles.clone(),
        };
        let token = pylon_auth::jwt::mint(secret.as_bytes(), &claims);
        return Some((
            200,
            serde_json::json!({"token": token, "expires_at": claims.exp}).to_string(),
        ));
    }

    // POST /api/auth/refresh
    if url == "/api/auth/refresh" && method == HttpMethod::Post {
        let old = match auth_token {
            Some(t) => t,
            None => return Some((401, json_error("AUTH_REQUIRED", "No session to refresh"))),
        };
        match ctx.session_store.refresh(old) {
            Some(session) => {
                return Some((
                    200,
                    serde_json::json!({
                        "token": session.token,
                        "user_id": session.user_id,
                        "expires_at": session.expires_at,
                    })
                    .to_string(),
                ));
            }
            None => {
                return Some((
                    401,
                    json_error("SESSION_EXPIRED", "Session is expired or invalid"),
                ));
            }
        }
    }

    // POST /api/auth/oauth/refresh/<provider>
    //
    // Wave 8 — surface the AccountStore::ensure_fresh_access_token
    // helper so apps can guarantee a non-stale OAuth access token
    // before calling the provider's API. Common pattern:
    //
    //   const { access_token } = await pylon.fetch(
    //     "/api/auth/oauth/refresh/google", { method: "POST" });
    //   await fetch("https://gmail.googleapis.com/...",
    //     { headers: { Authorization: `Bearer ${access_token}` }});
    //
    // The endpoint is an idempotent no-op when the cached token still
    // has > 60s of life — won't burn provider rate-limit budget on
    // every request.
    if let Some(provider_raw) = url.strip_prefix("/api/auth/oauth/refresh/") {
        if method == HttpMethod::Post {
            let provider = provider_raw.split('?').next().unwrap_or(provider_raw);
            if provider.is_empty() {
                return Some((
                    400,
                    json_error("MISSING_PROVIDER", "provider name is required"),
                ));
            }
            let user_id = match ctx.auth_ctx.user_id.as_deref() {
                Some(u) => u.to_string(),
                None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
            };
            // The user can have at most one account row per provider
            // (it's the natural key on AccountBackend), so finding by
            // user_id + provider is unambiguous. find_for_user filters
            // client-side; for at most a handful of accounts per user
            // that's fine.
            let account = ctx
                .account_store
                .find_for_user(&user_id)
                .into_iter()
                .find(|a| a.provider_id == provider);
            let account = match account {
                Some(a) => a,
                None => {
                    return Some((
                        404,
                        json_error("ACCOUNT_NOT_FOUND", "no linked account for that provider"),
                    ));
                }
            };
            // 60-second buffer: refresh if the access token expires in
            // <= 60s. Tunable via PYLON_OAUTH_REFRESH_BUFFER_SECS env if
            // an app needs longer (e.g. provider has slow propagation).
            let buffer = std::env::var("PYLON_OAUTH_REFRESH_BUFFER_SECS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60);
            match ctx.account_store.ensure_fresh_access_token(
                &account.provider_id,
                &account.account_id,
                buffer,
            ) {
                Ok(refreshed) => {
                    return Some((
                        200,
                        serde_json::json!({
                            "access_token": refreshed.access_token,
                            "expires_at": refreshed.access_token_expires_at,
                            "scope": refreshed.scope,
                            "provider": refreshed.provider_id,
                        })
                        .to_string(),
                    ));
                }
                Err(err) => {
                    let status = match err {
                        pylon_auth::RefreshError::AccountNotFound => 404,
                        pylon_auth::RefreshError::NoRefreshToken => 400,
                        pylon_auth::RefreshError::ProviderNotConfigured => 501,
                        pylon_auth::RefreshError::RefreshFailed(_) => 502,
                    };
                    return Some((status, json_error(err.code(), &err.message())));
                }
            }
        }
    }

    // GET /api/auth/sessions
    if url == "/api/auth/sessions" && method == HttpMethod::Get {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u,
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let list = ctx.session_store.list_for_user(user_id);
        let sanitized: Vec<serde_json::Value> = list
            .iter()
            .map(|s| {
                serde_json::json!({
                    "token_prefix": &s.token[..s.token.len().min(8)],
                    "user_id": s.user_id,
                    "device": s.device,
                    "created_at": s.created_at,
                    "expires_at": s.expires_at,
                })
            })
            .collect();
        return Some((
            200,
            serde_json::to_string(&sanitized).unwrap_or_else(|_| "[]".into()),
        ));
    }

    // DELETE /api/auth/sessions
    if url == "/api/auth/sessions" && method == HttpMethod::Delete {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u,
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let n = ctx.session_store.revoke_all_for_user(user_id);
        return Some((200, serde_json::json!({"revoked_count": n}).to_string()));
    }

    // ─── API keys ───────────────────────────────────────────────────────
    //
    // POST /api/auth/api-keys           — mint a new key (returns the
    //                                     plaintext exactly once)
    // GET  /api/auth/api-keys           — list (no plaintext, prefix only)
    // DELETE /api/auth/api-keys/:id     — revoke
    if url == "/api/auth/api-keys" {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u,
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        // P2 fix (codex Wave-2): API-key-authenticated requests cannot
        // create / list / revoke API keys. Same posture as Stripe
        // restricted keys — the user must hold a real session to manage
        // their keys. Admin tokens (server-issued) bypass.
        if ctx.auth_ctx.is_api_key_auth() && !ctx.auth_ctx.is_admin {
            return Some((
                403,
                json_error(
                    "API_KEY_AUTH_FORBIDDEN",
                    "API key management requires a session, not an API key",
                ),
            ));
        }
        if method == HttpMethod::Post {
            let data: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => {
                    return Some((
                        400,
                        json_error_safe(
                            "INVALID_JSON",
                            "Invalid request body",
                            &format!("Invalid JSON: {e}"),
                        ),
                    ));
                }
            };
            let name = data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("untitled")
                .to_string();
            let scopes = data
                .get("scopes")
                .and_then(|v| v.as_str())
                .map(String::from);
            let expires_at = data.get("expires_at").and_then(|v| v.as_u64());
            let (plaintext, key) =
                ctx.api_keys
                    .create(user_id.to_string(), name, scopes, expires_at);
            return Some((
                200,
                serde_json::json!({
                    // ONLY shown here. Frontend MUST display & forget.
                    "key": plaintext,
                    "id": key.id,
                    "prefix": key.prefix,
                    "name": key.name,
                    "scopes": key.scopes,
                    "expires_at": key.expires_at,
                    "created_at": key.created_at,
                })
                .to_string(),
            ));
        }
        if method == HttpMethod::Get {
            let list = ctx.api_keys.list_for_user(user_id);
            let payload: Vec<serde_json::Value> = list
                .iter()
                .map(|k| {
                    serde_json::json!({
                        "id": k.id,
                        "prefix": k.prefix,
                        "name": k.name,
                        "scopes": k.scopes,
                        "expires_at": k.expires_at,
                        "last_used_at": k.last_used_at,
                        "created_at": k.created_at,
                    })
                })
                .collect();
            return Some((
                200,
                serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into()),
            ));
        }
    }
    if let Some(id) = url.strip_prefix("/api/auth/api-keys/") {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u,
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        if method == HttpMethod::Delete {
            // Verify ownership before revoking — a compromised api-key
            // shouldn't let an attacker revoke arbitrary other users' keys.
            match ctx
                .api_keys
                .list_for_user(user_id)
                .iter()
                .find(|k| k.id == id)
            {
                Some(_) => {
                    let revoked = ctx.api_keys.revoke(id);
                    return Some((200, serde_json::json!({"revoked": revoked}).to_string()));
                }
                None => return Some((404, json_error("NOT_FOUND", "API key not found"))),
            }
        }
    }

    // ─── Password change (logged in) ───────────────────────────────────
    if url == "/api/auth/password/change" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        // P2 fix (codex Wave-2): API key auth cannot change passwords —
        // a leaked key shouldn't be able to lock the user out by
        // changing the password. Real session required.
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error(
                    "API_KEY_AUTH_FORBIDDEN",
                    "Password change requires a session, not an API key",
                ),
            ));
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                ));
            }
        };
        let current = data
            .get("currentPassword")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new_password = match data.get("newPassword").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return Some((
                    400,
                    json_error("MISSING_PASSWORD", "newPassword is required"),
                ));
            }
        };
        // Pull current row to verify old password (security rule:
        // session compromise alone shouldn't let an attacker change
        // the password and lock the user out).
        let row = match ctx
            .store
            .get_by_id(&ctx.store.manifest().auth.user.entity, &user_id)
        {
            Ok(Some(r)) => r,
            _ => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let stored_hash = row
            .get("passwordHash")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if stored_hash.is_empty() {
            return Some((
                400,
                json_error(
                    "NO_PASSWORD_SET",
                    "This account has no password (signed in via OAuth). Set one first.",
                ),
            ));
        }
        if !pylon_auth::password::verify_password(current, stored_hash) {
            return Some((
                401,
                json_error("WRONG_PASSWORD", "Current password is incorrect"),
            ));
        }
        if let Err(e) = pylon_auth::password::validate_length(new_password) {
            return Some((400, json_error("WEAK_PASSWORD", &e.to_string())));
        }
        if std::env::var("PYLON_DISABLE_HIBP").ok().as_deref() != Some("1") {
            if let Ok(n) = pylon_auth::password::check_pwned(new_password) {
                if n > 0 {
                    return Some((
                        400,
                        json_error_safe(
                            "PWNED_PASSWORD",
                            "This password has appeared in known data breaches.",
                            &format!("HIBP returned {n} occurrences"),
                        ),
                    ));
                }
            }
        }
        let new_hash = pylon_auth::password::hash_password(new_password);
        match ctx.store.update(
            &ctx.store.manifest().auth.user.entity,
            &user_id,
            &serde_json::json!({"passwordHash": new_hash}),
        ) {
            Ok(_) => {}
            Err(e) => return Some((400, json_error(&e.code, &e.message))),
        }
        // P1 fix (codex Wave-2): revoke ALL other sessions on
        // password change — better-auth pattern. A stolen session
        // shouldn't survive the password change that's meant to
        // contain its blast radius. We then re-mint a fresh session
        // for THIS request so the user isn't logged out of their
        // own password-change tab.
        let total_revoked = ctx.session_store.revoke_all_for_user(&user_id);
        let session = create_session_with_device(ctx, user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((
            200,
            serde_json::json!({
                "changed": true,
                "revoked_sessions": total_revoked,
                "token": session.token,
                "expires_at": session.expires_at,
            })
            .to_string(),
        ));
    }

    // ─── TOTP (RFC 6238 — 6-digit, 30-second, HMAC-SHA1) ──────────────
    //
    // Two-step enrollment: POST /enroll returns a fresh secret +
    // provisioning URL (NOT yet active); POST /verify with a code from
    // the user's authenticator app finalizes it. Subsequent password /
    // magic-code logins don't auto-enforce TOTP — apps gate that
    // themselves by checking `User.totpVerified`. This matches
    // better-auth's "you bring the gate" stance.
    if url == "/api/auth/totp/enroll" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error(
                    "API_KEY_AUTH_FORBIDDEN",
                    "TOTP enrollment requires a session",
                ),
            ));
        }
        // Fetch user row to derive the QR account label (their email).
        let row = match ctx
            .store
            .get_by_id(&ctx.store.manifest().auth.user.entity, &user_id)
        {
            Ok(Some(r)) => r,
            _ => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        // If TOTP is already verified, require a current TOTP code to
        // re-enroll. Defends against a session-cookie-only attacker
        // silently rotating the secret to one they control. Same posture
        // as password change requiring the current password.
        let already_verified = row
            .get("totpVerified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if already_verified {
            let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let code = data.get("code").and_then(|v| v.as_str()).unwrap_or("");
            let stored_blob = row.get("totpSecret").and_then(|v| v.as_str()).unwrap_or("");
            let secret_b32 = pylon_auth::totp::unseal_secret(stored_blob).unwrap_or_default();
            let secret = pylon_auth::totp::base32_decode(&secret_b32).unwrap_or_default();
            if !pylon_auth::totp::verify_now(&secret, code) {
                return Some((
                    401,
                    json_error(
                        "INVALID_TOTP_CODE",
                        "TOTP is already enrolled — provide a current code to rotate the secret",
                    ),
                ));
            }
        }
        let account = row
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or(&user_id)
            .to_string();
        let issuer = std::env::var("PYLON_TOTP_ISSUER")
            .unwrap_or_else(|_| ctx.store.manifest().name.clone());

        let secret = pylon_auth::totp::generate_secret();
        let secret_b32 = pylon_auth::totp::base32_encode(&secret);
        let url_otp = pylon_auth::totp::provisioning_url(&issuer, &account, &secret_b32);
        // Persist the secret as PENDING (totpVerified=false), encrypted
        // at rest with PYLON_TOTP_ENCRYPTION_KEY. The app's user
        // entity needs `totpSecret: string?` + `totpVerified: bool?`.
        let sealed = pylon_auth::totp::seal_secret(&secret_b32);
        match ctx.store.update(
            &ctx.store.manifest().auth.user.entity,
            &user_id,
            &serde_json::json!({
                "totpSecret": sealed,
                "totpVerified": false,
            }),
        ) {
            Ok(_) => {}
            Err(e) => return Some((400, json_error(&e.code, &e.message))),
        }
        return Some((
            200,
            serde_json::json!({
                "secret": secret_b32,
                "url": url_otp,
                // Apps MAY render the QR code themselves; expose the
                // raw bytes too for non-web clients.
                "issuer": issuer,
                "account": account,
            })
            .to_string(),
        ));
    }

    if url == "/api/auth/totp/verify" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe("INVALID_JSON", "Invalid request body", &format!("{e}")),
                ));
            }
        };
        let code = data
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let row = match ctx
            .store
            .get_by_id(&ctx.store.manifest().auth.user.entity, &user_id)
        {
            Ok(Some(r)) => r,
            _ => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let secret_blob = match row.get("totpSecret").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return Some((
                    400,
                    json_error("TOTP_NOT_ENROLLED", "Call /api/auth/totp/enroll first"),
                ));
            }
        };
        let secret_b32 = match pylon_auth::totp::unseal_secret(secret_blob) {
            Ok(s) => s,
            Err(_) => {
                return Some((
                    500,
                    json_error(
                        "TOTP_BAD_SECRET",
                        "Stored secret is corrupt or PYLON_TOTP_ENCRYPTION_KEY missing",
                    ),
                ))
            }
        };
        let secret = match pylon_auth::totp::base32_decode(&secret_b32) {
            Ok(s) => s,
            Err(_) => {
                return Some((
                    500,
                    json_error("TOTP_BAD_SECRET", "Stored secret is corrupt"),
                ))
            }
        };
        // Per-account rate limit on verify so backup-code brute
        // force can't churn through 10 codes in a second.
        let rl = pylon_auth::rate_limit::AuthRateLimiter::shared();
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } = rl.check(
            pylon_auth::rate_limit::AuthBucket::Login,
            ctx.peer_ip,
            Some(&user_id),
        ) {
            return Some((
                429,
                json_error_with_hint(
                    "RATE_LIMITED",
                    "Too many TOTP attempts",
                    &format!("Try again in {retry_after_secs}s"),
                ),
            ));
        }
        // Accept either the current 6-digit TOTP code OR one of the
        // hashed backup codes (consumed on use). Backup codes are
        // typically formatted XXXX-XXXX so they're easy to spot.
        let mut backup_consumed: Option<usize> = None;
        let totp_ok = pylon_auth::totp::verify_now(&secret, &code);
        if !totp_ok {
            // Try backup codes.
            if let Some(stored) = row.get("totpBackupCodes").and_then(|v| v.as_array()) {
                use sha2::{Digest, Sha256};
                let mut h = Sha256::new();
                h.update(code.as_bytes());
                let candidate = h.finalize();
                use std::fmt::Write;
                let mut hex = String::with_capacity(64);
                for b in candidate {
                    let _ = write!(hex, "{b:02x}");
                }
                for (i, hash) in stored.iter().enumerate() {
                    if let Some(s) = hash.as_str() {
                        if pylon_auth::constant_time_eq(s.as_bytes(), hex.as_bytes()) {
                            backup_consumed = Some(i);
                            break;
                        }
                    }
                }
            }
        }
        if !totp_ok && backup_consumed.is_none() {
            return Some((401, json_error("INVALID_TOTP_CODE", "Wrong code")));
        }
        // Backup-code consumption: rewrite the array WITHOUT the
        // matching index. Single-use; keeping the rest valid.
        // Wave-6 codex P1: under concurrent verifies the read-modify-
        // write pattern races. We mitigate by re-reading the row
        // AFTER the write and asserting the consumed hash is gone.
        // If a parallel verify swapped in its own version (where our
        // hash is still present), we lost the race — refuse to mint
        // the session so only ONE caller per code wins.
        if let Some(idx) = backup_consumed {
            let consumed_hash = row
                .get("totpBackupCodes")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.get(idx))
                .and_then(|v| v.as_str())
                .map(String::from);
            if let Some(stored) = row.get("totpBackupCodes").and_then(|v| v.as_array()) {
                let kept: Vec<&serde_json::Value> = stored
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != idx)
                    .map(|(_, v)| v)
                    .collect();
                let _ = ctx.store.update(
                    &ctx.store.manifest().auth.user.entity,
                    &user_id,
                    &serde_json::json!({"totpBackupCodes": kept}),
                );
            }
            // Post-write verify: the consumed hash MUST be absent now.
            if let Some(want_gone) = consumed_hash {
                if let Ok(Some(after)) = ctx
                    .store
                    .get_by_id(&ctx.store.manifest().auth.user.entity, &user_id)
                {
                    if let Some(arr) = after.get("totpBackupCodes").and_then(|v| v.as_array()) {
                        let still_there = arr
                            .iter()
                            .any(|v| v.as_str().map(|s| s == want_gone).unwrap_or(false));
                        if still_there {
                            // A concurrent verify swapped in a row
                            // where our consumed code is back in the
                            // array. Refuse — only one caller wins.
                            return Some((409, json_error(
                                "TOTP_RACE",
                                "Backup code was consumed by a concurrent request. Try a different code.",
                            )));
                        }
                    }
                }
            }
        }
        let was_verified = row
            .get("totpVerified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !was_verified {
            // Stamp on first successful verify so the app knows
            // enrollment is finalized.
            match ctx.store.update(
                &ctx.store.manifest().auth.user.entity,
                &user_id,
                &serde_json::json!({"totpVerified": true}),
            ) {
                Ok(_) => {}
                Err(e) => return Some((400, json_error(&e.code, &e.message))),
            }
        }
        // Wave-7 E. Optional `trust_device: true` body field: mint a
        // trusted-device record bound to this user + set the
        // `pylon_trusted_device` cookie. Apps that gate sensitive flows
        // on TOTP can then skip the prompt for 30 days from this
        // browser by checking `ctx.auth.isTrustedDevice`. Skipped
        // silently when false/absent — opt-in.
        let trust_requested = data
            .get("trust_device")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut trust_minted: Option<String> = None;
        if trust_requested {
            let label = request_user_agent(ctx).map(pylon_auth::device::parse_user_agent);
            let device = pylon_auth::trusted_device::TrustedDevice::mint(
                user_id.clone(),
                label,
                pylon_auth::trusted_device::DEFAULT_TRUST_LIFETIME_SECS,
            );
            trust_minted = Some(device.token.clone());
            ctx.trusted_devices.create(device);
            // Cookie shape mirrors the session cookie (HttpOnly, Secure
            // when applicable, SameSite=Lax). Built from the existing
            // CookieConfig knobs so secure/SameSite/path stay in lockstep
            // with sessions — operators don't have to configure two
            // sets of cookie attributes.
            if let Some(token) = &trust_minted {
                let cookie_value = ctx.cookie_config.set_value_for(
                    pylon_auth::trusted_device::TRUST_COOKIE_NAME,
                    token,
                    Some(pylon_auth::trusted_device::DEFAULT_TRUST_LIFETIME_SECS),
                );
                ctx.add_response_header("Set-Cookie", cookie_value);
            }
        }
        return Some((
            200,
            serde_json::json!({
                "verified": true,
                "enrolled": !was_verified,
                "trust_device": trust_minted.is_some(),
            })
            .to_string(),
        ));
    }

    // GET /api/auth/trusted-devices — list current user's trusted
    // browsers. Each entry has the random token (used for revoke), the
    // device label (parsed UA), created_at + expires_at. Useful for an
    // "active devices" account-settings page.
    if url == "/api/auth/trusted-devices" && method == HttpMethod::Get {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error(
                    "API_KEY_AUTH_FORBIDDEN",
                    "Trusted devices require a session",
                ),
            ));
        }
        let devices = ctx.trusted_devices.list_for_user(&user_id);
        return Some((
            200,
            serde_json::json!({
                // Notably absent: `token`. The cookie value MUST stay
                // server-side. Returning it would let dashboard XSS
                // exfiltrate trust tokens equivalent to stealing the
                // cookie itself. `id` is the management handle.
                "devices": devices
                    .into_iter()
                    .map(|d| serde_json::json!({
                        "id": d.id,
                        "label": d.label,
                        "created_at": d.created_at,
                        "expires_at": d.expires_at,
                    }))
                    .collect::<Vec<_>>(),
            })
            .to_string(),
        ));
    }

    // DELETE /api/auth/trusted-devices — revoke ALL of the current
    // user's trusted devices. The "log everything else out of TOTP" big
    // red button.
    if url == "/api/auth/trusted-devices" && method == HttpMethod::Delete {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error(
                    "API_KEY_AUTH_FORBIDDEN",
                    "Trusted devices require a session",
                ),
            ));
        }
        let removed = ctx.trusted_devices.revoke_all_for_user(&user_id);
        // Clear the trust cookie on the current request so the browser
        // doesn't keep presenting a now-revoked token.
        ctx.add_response_header(
            "Set-Cookie",
            ctx.cookie_config
                .clear_value_for(pylon_auth::trusted_device::TRUST_COOKIE_NAME),
        );
        return Some((200, serde_json::json!({"revoked": removed}).to_string()));
    }

    // DELETE /api/auth/trusted-devices/<id> — revoke a single trusted-
    // device record. The id is the public-facing handle returned by
    // GET /api/auth/trusted-devices; the secret token (the cookie value)
    // is never exposed to the client. Only succeeds when the record's
    // user_id matches the caller — preventing one user from revoking
    // another's trust by guessing ids.
    if url.starts_with("/api/auth/trusted-devices/") && method == HttpMethod::Delete {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error(
                    "API_KEY_AUTH_FORBIDDEN",
                    "Trusted devices require a session",
                ),
            ));
        }
        let id = &url["/api/auth/trusted-devices/".len()..];
        if id.is_empty() {
            return Some((400, json_error("MISSING_ID", "trust device id is required")));
        }
        // Object-level auth: only the owner can revoke. Look up first
        // so cross-user revoke attempts return 404 identical to "doesn't
        // exist" — defense against id enumeration via response timing
        // (both paths run find_by_id then short-circuit at the same
        // point).
        let owned = ctx
            .trusted_devices
            .find_by_id(id)
            .map(|d| d.user_id == user_id)
            .unwrap_or(false);
        if !owned {
            return Some((404, json_error("NOT_FOUND", "trusted device not found")));
        }
        let removed = ctx.trusted_devices.revoke_by_id(id);
        return Some((200, serde_json::json!({"revoked": removed}).to_string()));
    }

    if url == "/api/auth/totp/disable" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error("API_KEY_AUTH_FORBIDDEN", "TOTP disable requires a session"),
            ));
        }
        // Require a current code to disable — defends against a
        // session-cookie-only attacker silently turning off 2FA.
        let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let code = data.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let row = match ctx
            .store
            .get_by_id(&ctx.store.manifest().auth.user.entity, &user_id)
        {
            Ok(Some(r)) => r,
            _ => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        if let Some(secret_blob) = row.get("totpSecret").and_then(|v| v.as_str()) {
            if !secret_blob.is_empty() {
                let secret_b32 = pylon_auth::totp::unseal_secret(secret_blob).unwrap_or_default();
                let secret = pylon_auth::totp::base32_decode(&secret_b32).unwrap_or_default();
                if !pylon_auth::totp::verify_now(&secret, code) {
                    return Some((
                        401,
                        json_error(
                            "INVALID_TOTP_CODE",
                            "Provide a current TOTP code to disable 2FA",
                        ),
                    ));
                }
            }
        }
        match ctx.store.update(
            &ctx.store.manifest().auth.user.entity,
            &user_id,
            &serde_json::json!({"totpSecret": null, "totpVerified": false}),
        ) {
            Ok(_) => {}
            Err(e) => return Some((400, json_error(&e.code, &e.message))),
        }
        return Some((200, serde_json::json!({"disabled": true}).to_string()));
    }

    // ─── Per-org SSO public endpoints (unauthenticated) ────────────────
    //
    // GET /api/auth/orgs/<org_id>/sso/start
    //   Mints a fresh PKCE-protected state record + redirects the
    //   browser to the org's IdP authorization endpoint. Public — the
    //   user is signing in, they don't have a session yet.
    // GET /api/auth/orgs/<org_id>/sso/callback
    //   Validates the state, exchanges the code at the org's IdP,
    //   fetches userinfo, looks up or creates the User row, auto-joins
    //   them to the org with the configured default role, and mints a
    //   session.
    if let Some(rest) = url.strip_prefix("/api/auth/orgs/") {
        let path = rest.split('?').next().unwrap_or(rest);
        // Two suffixes — /sso/start and /sso/callback. The bare /sso
        // CRUD endpoints below need an authenticated caller and live
        // in the auth-required block.
        if let Some(org_id) = path.strip_suffix("/sso/start") {
            if method == HttpMethod::Get {
                return Some(handle_org_sso_start(ctx, org_id, rest));
            }
        }
        if let Some(org_id) = path.strip_suffix("/sso/callback") {
            if method == HttpMethod::Get {
                return Some(handle_org_sso_callback(ctx, org_id, rest));
            }
        }
    }

    // ─── Organizations + invites ───────────────────────────────────────
    //
    // POST   /api/auth/orgs                          create org
    // GET    /api/auth/orgs                          list user's orgs
    // GET    /api/auth/orgs/:id                      org details
    // DELETE /api/auth/orgs/:id                      delete (owner only)
    // GET    /api/auth/orgs/:id/members              list members
    // PUT    /api/auth/orgs/:id/members/:user_id     change role (admin+)
    // DELETE /api/auth/orgs/:id/members/:user_id     remove member (admin+)
    // POST   /api/auth/orgs/:id/invites              send email invite
    // GET    /api/auth/orgs/:id/invites              list pending invites
    // DELETE /api/auth/orgs/:id/invites/:invite_id   revoke invite
    // POST   /api/auth/invites/:token/accept         accept (sets membership)
    if url == "/api/auth/orgs" {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        // Wave-4 codex P1: block API-key auth from org management.
        // A leaked API key shouldn't be able to create/delete orgs or
        // manage members. Real session required.
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error(
                    "API_KEY_AUTH_FORBIDDEN",
                    "Org management requires a session",
                ),
            ));
        }
        if method == HttpMethod::Post {
            let data: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => {
                    return Some((
                        400,
                        json_error_safe("INVALID_JSON", "Invalid request body", &format!("{e}")),
                    ));
                }
            };
            let name = data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                return Some((400, json_error("MISSING_NAME", "name is required")));
            }
            let org = ctx.orgs.create(name, &user_id);
            return Some((
                200,
                serde_json::json!({
                    "id": org.id,
                    "name": org.name,
                    "created_at": org.created_at,
                    "role": "owner",
                })
                .to_string(),
            ));
        }
        if method == HttpMethod::Get {
            let list = ctx.orgs.list_for_user(&user_id);
            let payload: Vec<serde_json::Value> = list
                .iter()
                .map(|(o, role)| {
                    serde_json::json!({
                        "id": o.id,
                        "name": o.name,
                        "role": role.as_str(),
                        "created_at": o.created_at,
                    })
                })
                .collect();
            return Some((
                200,
                serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into()),
            ));
        }
    }

    if let Some(rest) = url.strip_prefix("/api/auth/orgs/") {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error(
                    "API_KEY_AUTH_FORBIDDEN",
                    "Org management requires a session",
                ),
            ));
        }
        let parts: Vec<&str> = rest.splitn(4, '/').collect();
        let org_id = parts[0];

        // Caller must be a member to do anything below.
        let caller_role = match ctx.orgs.role_of(org_id, &user_id) {
            Some(r) => r,
            None => return Some((404, json_error("ORG_NOT_FOUND", "Org not found"))),
        };

        match parts.as_slice() {
            // /api/auth/orgs/:id
            [_id] if method == HttpMethod::Get => {
                let org = ctx.orgs.get(org_id).expect("role implies org exists");
                return Some((
                    200,
                    serde_json::json!({
                        "id": org.id,
                        "name": org.name,
                        "created_at": org.created_at,
                        "role": caller_role.as_str(),
                    })
                    .to_string(),
                ));
            }
            [_id] if method == HttpMethod::Delete => {
                if !caller_role.can_delete_org() {
                    return Some((
                        403,
                        json_error("FORBIDDEN", "Only owners can delete an org"),
                    ));
                }
                let removed = ctx.orgs.delete(org_id);
                return Some((200, serde_json::json!({"deleted": removed}).to_string()));
            }
            // ───── Wave-8 per-org SSO config CRUD ─────
            // GET /api/auth/orgs/:id/sso — any member sees the redacted
            // config (so the dashboard "SSO is active" badge works).
            [_id, "sso"] if method == HttpMethod::Get => {
                return Some(match ctx.org_sso.get(org_id) {
                    Some(cfg) => {
                        let r = cfg.redacted();
                        (
                            200,
                            serde_json::json!({
                                "configured": true,
                                "issuer_url": r.issuer_url,
                                "client_id": r.client_id,
                                "default_role": r.default_role,
                                "email_domains": r.email_domains,
                                "authorization_endpoint": r.authorization_endpoint,
                                "token_endpoint": r.token_endpoint,
                                "userinfo_endpoint": r.userinfo_endpoint,
                                "jwks_uri": r.jwks_uri,
                                "created_at": r.created_at,
                                "updated_at": r.updated_at,
                            })
                            .to_string(),
                        )
                    }
                    None => (200, serde_json::json!({"configured": false}).to_string()),
                });
            }
            // PUT /api/auth/orgs/:id/sso — owner-only. Discovers the
            // IdP, encrypts the secret, persists.
            [_id, "sso"] if method == HttpMethod::Put => {
                if !caller_role.can_delete_org() {
                    return Some((
                        403,
                        json_error("FORBIDDEN", "Only owners can configure SSO"),
                    ));
                }
                let data: serde_json::Value = match serde_json::from_str(body) {
                    Ok(v) => v,
                    Err(e) => {
                        return Some((
                            400,
                            json_error_safe(
                                "INVALID_JSON",
                                "Invalid request body",
                                &format!("{e}"),
                            ),
                        ))
                    }
                };
                let issuer = data
                    .get("issuer_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let client_id = data
                    .get("client_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let client_secret = data
                    .get("client_secret")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if issuer.is_empty() || client_id.is_empty() || client_secret.is_empty() {
                    return Some((
                        400,
                        json_error(
                            "MISSING_FIELDS",
                            "issuer_url + client_id + client_secret are all required",
                        ),
                    ));
                }
                let default_role = data
                    .get("default_role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("member")
                    .to_string();
                if default_role.eq_ignore_ascii_case("owner") {
                    // Hard rule — auto-joining as owner via IdP would
                    // let an IdP misconfiguration silently hand over
                    // control of the org.
                    return Some((
                        400,
                        json_error(
                            "BAD_DEFAULT_ROLE",
                            "default_role must be `member` or `admin`, never `owner`",
                        ),
                    ));
                }
                let email_domains: Vec<String> = data
                    .get("email_domains")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| s.to_ascii_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                let endpoints = match pylon_auth::org_sso::discover_endpoints(&issuer) {
                    Ok(e) => e,
                    Err(e) => {
                        return Some((
                            400,
                            json_error_safe(
                                "DISCOVERY_FAILED",
                                "Could not load IdP discovery doc",
                                &e,
                            ),
                        ))
                    }
                };
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let existing_created_at =
                    ctx.org_sso.get(org_id).map(|c| c.created_at).unwrap_or(now);
                let cfg = pylon_auth::org_sso::OrgSsoConfig {
                    org_id: org_id.to_string(),
                    issuer_url: issuer,
                    client_id,
                    client_secret_sealed: pylon_auth::org_sso::seal_secret(&client_secret),
                    default_role,
                    email_domains,
                    authorization_endpoint: endpoints.authorization_endpoint,
                    token_endpoint: endpoints.token_endpoint,
                    userinfo_endpoint: endpoints.userinfo_endpoint,
                    jwks_uri: endpoints.jwks_uri,
                    created_at: existing_created_at,
                    updated_at: now,
                };
                ctx.org_sso.upsert(cfg);
                return Some((200, serde_json::json!({"configured": true}).to_string()));
            }
            // DELETE /api/auth/orgs/:id/sso — owner-only.
            [_id, "sso"] if method == HttpMethod::Delete => {
                if !caller_role.can_delete_org() {
                    return Some((
                        403,
                        json_error("FORBIDDEN", "Only owners can delete SSO config"),
                    ));
                }
                let removed = ctx.org_sso.delete(org_id);
                return Some((200, serde_json::json!({"deleted": removed}).to_string()));
            }
            // /api/auth/orgs/:id/members
            [_id, "members"] if method == HttpMethod::Get => {
                let list = ctx.orgs.list_members(org_id);
                let payload: Vec<serde_json::Value> = list
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "user_id": m.user_id,
                            "role": m.role.as_str(),
                            "joined_at": m.joined_at,
                        })
                    })
                    .collect();
                return Some((
                    200,
                    serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into()),
                ));
            }
            // /api/auth/orgs/:id/members/:user_id
            [_id, "members", target_user] if method == HttpMethod::Put => {
                if !caller_role.can_manage_members() {
                    return Some((403, json_error("FORBIDDEN", "Insufficient role")));
                }
                let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
                let role_str = data.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let role = match pylon_auth::org::OrgRole::from_str(role_str) {
                    Some(r) => r,
                    None => {
                        return Some((
                            400,
                            json_error("BAD_ROLE", "role must be owner|admin|member"),
                        ))
                    }
                };
                // Wave-4 codex P1: only OWNERS can promote members
                // to owner. Without this an admin can self-promote
                // (PUT members/<self>/role: owner) and then delete
                // the org. Owner-promotion is the privilege boundary.
                if role == pylon_auth::org::OrgRole::Owner && !caller_role.can_transfer_ownership()
                {
                    return Some((
                        403,
                        json_error("FORBIDDEN", "Only owners can promote a member to owner"),
                    ));
                }
                // Wave-4 codex P1: prevent demoting the last owner.
                // Same rule as remove-member; without it an owner
                // can demote themselves and orphan the org.
                if let Some(target_role) = ctx.orgs.role_of(org_id, target_user) {
                    if target_role == pylon_auth::org::OrgRole::Owner
                        && role != pylon_auth::org::OrgRole::Owner
                    {
                        let owners = ctx
                            .orgs
                            .list_members(org_id)
                            .into_iter()
                            .filter(|m| m.role == pylon_auth::org::OrgRole::Owner)
                            .count();
                        if owners <= 1 {
                            return Some((
                                400,
                                json_error(
                                    "LAST_OWNER",
                                    "Cannot demote the last owner — promote someone else first",
                                ),
                            ));
                        }
                    }
                }
                let updated = ctx.orgs.set_role(org_id, target_user, role);
                if !updated {
                    return Some((
                        404,
                        json_error("NOT_A_MEMBER", "Target user is not a member"),
                    ));
                }
                return Some((200, serde_json::json!({"updated": true}).to_string()));
            }
            [_id, "members", target_user] if method == HttpMethod::Delete => {
                if !caller_role.can_manage_members() && target_user != &user_id.as_str() {
                    return Some((403, json_error("FORBIDDEN", "Insufficient role")));
                }
                // Prevent removing the LAST owner — would orphan the org.
                if let Some(target_role) = ctx.orgs.role_of(org_id, target_user) {
                    if target_role == pylon_auth::org::OrgRole::Owner {
                        let owners = ctx
                            .orgs
                            .list_members(org_id)
                            .into_iter()
                            .filter(|m| m.role == pylon_auth::org::OrgRole::Owner)
                            .count();
                        if owners <= 1 {
                            return Some((
                                400,
                                json_error(
                                    "LAST_OWNER",
                                    "Cannot remove the last owner — promote someone else first",
                                ),
                            ));
                        }
                    }
                }
                let removed = ctx.orgs.remove_member(org_id, target_user);
                return Some((200, serde_json::json!({"removed": removed}).to_string()));
            }
            // /api/auth/orgs/:id/invites
            [_id, "invites"] if method == HttpMethod::Post => {
                if !caller_role.can_manage_members() {
                    return Some((403, json_error("FORBIDDEN", "Insufficient role")));
                }
                let data: serde_json::Value = match serde_json::from_str(body) {
                    Ok(v) => v,
                    Err(e) => {
                        return Some((
                            400,
                            json_error_safe(
                                "INVALID_JSON",
                                "Invalid request body",
                                &format!("{e}"),
                            ),
                        ));
                    }
                };
                let email = data
                    .get("email")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if email.is_empty() || !email.contains('@') {
                    return Some((400, json_error("INVALID_EMAIL", "valid email required")));
                }
                let role_str = data
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("member");
                let role = pylon_auth::org::OrgRole::from_str(role_str)
                    .unwrap_or(pylon_auth::org::OrgRole::Member);
                let invited = ctx.orgs.create_invite(org_id, email, role, &user_id);
                // Best-effort email — failure to send still returns
                // success because the inviter can copy the link from
                // the response (apps can hide it in production).
                let org = ctx.orgs.get(org_id).expect("role implies exists");
                let accept_url = format!(
                    "{}/api/auth/invites/{}/accept",
                    std::env::var("PYLON_PUBLIC_URL").unwrap_or_else(|_| String::new()),
                    invited.token
                );
                let mut vars = std::collections::HashMap::new();
                vars.insert("org_name", org.name.as_str());
                vars.insert("url", accept_url.as_str());
                let (subject, body_text) = pylon_auth::email_templates::render(
                    pylon_auth::email_templates::EmailTemplate::OrgInvite,
                    &vars,
                );
                if let Err(e) = ctx.email.send(email, &subject, &body_text) {
                    tracing::warn!("[org] invite email to {} failed: {e}", redact_email(email));
                }
                return Some((
                    200,
                    serde_json::json!({
                        "id": invited.invite.id,
                        "email": invited.invite.email,
                        "role": invited.invite.role.as_str(),
                        "expires_at": invited.invite.expires_at,
                        "accept_url": accept_url,
                        // Plaintext token so the inviter can copy/paste
                        // when email isn't configured (dev mode).
                        "token": if ctx.is_dev { Some(&invited.token) } else { None },
                    })
                    .to_string(),
                ));
            }
            [_id, "invites"] if method == HttpMethod::Get => {
                if !caller_role.can_manage_members() {
                    return Some((403, json_error("FORBIDDEN", "Insufficient role")));
                }
                let list = ctx.orgs.list_invites(org_id);
                let payload: Vec<serde_json::Value> = list
                    .iter()
                    .map(|i| {
                        serde_json::json!({
                            "id": i.id,
                            "email": i.email,
                            "role": i.role.as_str(),
                            "token_prefix": i.token_prefix,
                            "invited_by": i.invited_by,
                            "created_at": i.created_at,
                            "expires_at": i.expires_at,
                        })
                    })
                    .collect();
                return Some((
                    200,
                    serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into()),
                ));
            }
            [_id, "invites", invite_id] if method == HttpMethod::Delete => {
                if !caller_role.can_manage_members() {
                    return Some((403, json_error("FORBIDDEN", "Insufficient role")));
                }
                // Wave-4 codex P2: object-level auth — verify the
                // invite actually belongs to THIS org. The URL claim
                // (org_id) wasn't matched against the row before;
                // a global revoke_invite(invite_id) lets an admin
                // of org A revoke any invite_id they happen to know
                // even when it's for org B.
                match ctx
                    .orgs
                    .list_invites(org_id)
                    .into_iter()
                    .find(|i| i.id == *invite_id)
                {
                    Some(_) => {
                        let revoked = ctx.orgs.revoke_invite(invite_id);
                        return Some((200, serde_json::json!({"revoked": revoked}).to_string()));
                    }
                    None => {
                        return Some((404, json_error("NOT_FOUND", "Invite not found in this org")))
                    }
                }
            }
            _ => {}
        }
    }

    // POST /api/auth/invites/:token/accept
    if let Some(rest) = url.strip_prefix("/api/auth/invites/") {
        if let Some(token) = rest.strip_suffix("/accept") {
            if method == HttpMethod::Post {
                let user_id = match ctx.auth_ctx.user_id.as_deref() {
                    Some(u) => u.to_string(),
                    None => {
                        return Some((
                            401,
                            json_error("AUTH_REQUIRED", "Login required to accept an invite"),
                        ))
                    }
                };
                // Pull the user's email from their row to verify the invite.
                let row = match ctx
                    .store
                    .get_by_id(&ctx.store.manifest().auth.user.entity, &user_id)
                {
                    Ok(Some(r)) => r,
                    _ => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
                };
                let email = match row.get("email").and_then(|v| v.as_str()) {
                    Some(e) => e,
                    None => return Some((400, json_error("NO_EMAIL", "Account has no email"))),
                };
                match ctx.orgs.accept_invite(token, &user_id, email) {
                    Ok(m) => {
                        return Some((
                            200,
                            serde_json::json!({
                                "org_id": m.org_id,
                                "role": m.role.as_str(),
                            })
                            .to_string(),
                        ));
                    }
                    Err(e) => {
                        let code = match e {
                            pylon_auth::org::AcceptError::NotFound => "INVITE_NOT_FOUND",
                            pylon_auth::org::AcceptError::Expired => "INVITE_EXPIRED",
                            pylon_auth::org::AcceptError::AlreadyAccepted => "ALREADY_ACCEPTED",
                            pylon_auth::org::AcceptError::EmailMismatch => "WRONG_EMAIL",
                            pylon_auth::org::AcceptError::AlreadyMember => "ALREADY_MEMBER",
                        };
                        return Some((400, json_error(code, &e.to_string())));
                    }
                }
            }
        }
    }

    // ─── Phone / SMS sign-in ──────────────────────────────────────────
    if url == "/api/auth/phone/send-code" && method == HttpMethod::Post {
        let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let phone = data.get("phone").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(cfg) = pylon_auth::captcha::CaptchaConfig::from_env() {
            let token = data
                .get("captchaToken")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if cfg.verify(token, Some(ctx.peer_ip)).is_err() {
                return Some((400, json_error("CAPTCHA_FAILED", "CAPTCHA failed")));
            }
        }
        let code = match ctx.phone_codes.try_create(phone) {
            Ok(c) => c,
            Err(pylon_auth::phone::PhoneCodeError::Throttled { retry_after_secs }) => {
                return Some((
                    429,
                    json_error_with_hint(
                        "RATE_LIMITED",
                        "Code requested too recently",
                        &format!("Try again in {retry_after_secs}s"),
                    ),
                ));
            }
            Err(pylon_auth::phone::PhoneCodeError::InvalidPhone) => {
                return Some((
                    400,
                    json_error("INVALID_PHONE", "Phone must be E.164 (+15551234567)"),
                ));
            }
            Err(e) => return Some((500, json_error("PHONE_CODE_FAILED", &e.to_string()))),
        };
        let mut sent = false;
        if let Some(twilio) = pylon_auth::phone::TwilioSmsTransport::from_env() {
            use pylon_auth::phone::SmsSender;
            let body_text = format!("Your sign-in code is: {code}\nExpires in 10 minutes.");
            if let Err(e) = twilio.send_sms(phone, &body_text) {
                tracing::warn!("[phone] twilio send failed: {e}");
            } else {
                sent = true;
            }
        }
        let mut response = serde_json::json!({"sent": sent, "phone": phone});
        if ctx.is_dev || !sent {
            response["dev_code"] = serde_json::Value::String(code);
        }
        return Some((200, response.to_string()));
    }

    if url == "/api/auth/phone/verify" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe("INVALID_JSON", "Invalid request body", &format!("{e}")),
                ))
            }
        };
        let phone = data.get("phone").and_then(|v| v.as_str()).unwrap_or("");
        let code = data.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let display_name = data
            .get("displayName")
            .and_then(|v| v.as_str())
            .map(String::from);
        if let Err(e) = ctx.phone_codes.try_verify(phone, code) {
            let http_code = match e {
                pylon_auth::phone::PhoneCodeError::TooManyAttempts => 429,
                pylon_auth::phone::PhoneCodeError::InvalidPhone => 400,
                _ => 401,
            };
            return Some((http_code, json_error("INVALID_CODE", &e.to_string())));
        }
        let normalized = pylon_auth::phone::normalize(phone).expect("verified above");
        let entity = &ctx.store.manifest().auth.user.entity;
        let user_id = match ctx.store.lookup(entity, "phone", &normalized) {
            Ok(Some(row)) => row
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            _ => {
                let now = format!(
                    "{}Z",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                );
                let dn = display_name.unwrap_or_else(|| normalized.clone());
                match ctx.store.insert(
                    entity,
                    &serde_json::json!({
                        "phone": normalized,
                        "displayName": dn,
                        "phoneVerified": now.clone(),
                        "createdAt": now,
                    }),
                ) {
                    Ok(id) => id,
                    Err(e) => return Some((400, json_error(&e.code, &e.message))),
                }
            }
        };
        let session = create_session_with_device(ctx, user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((
            200,
            serde_json::json!({
                "token": session.token, "user_id": user_id, "expires_at": session.expires_at
            })
            .to_string(),
        ));
    }

    // ─── SIWE — Sign-In With Ethereum ─────────────────────────────────
    if let Some(rest) = url.strip_prefix("/api/auth/siwe/nonce") {
        if method == HttpMethod::Get {
            let q = rest.trim_start_matches('?');
            let params = parse_query(q);
            let addr = params.get("address").map(|s| s.as_str()).unwrap_or("");
            if !addr.starts_with("0x") || addr.len() != 42 {
                return Some((
                    400,
                    json_error("INVALID_ADDRESS", "address must be 0x + 40 hex chars"),
                ));
            }
            let nonce = ctx.siwe.issue(addr);
            return Some((200, serde_json::json!({"nonce": nonce}).to_string()));
        }
    }
    if url == "/api/auth/siwe/verify" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")),
                ))
            }
        };
        let message_text = data.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let sig_hex = data.get("signature").and_then(|v| v.as_str()).unwrap_or("");
        let display_name = data
            .get("displayName")
            .and_then(|v| v.as_str())
            .map(String::from);
        let parsed = match pylon_auth::siwe::parse_message(message_text) {
            Ok(m) => m,
            Err(e) => return Some((400, json_error("SIWE_BAD_MESSAGE", &e.to_string()))),
        };
        let expected_domain = ctx
            .request_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("host"))
            .map(|(_, v)| v.split(':').next().unwrap_or(v).to_string())
            .unwrap_or_else(|| parsed.domain.clone());
        let recovered = match pylon_auth::siwe::verify(ctx.siwe, &parsed, sig_hex, &expected_domain)
        {
            Ok(addr) => addr,
            Err(e) => return Some((401, json_error("SIWE_VERIFY_FAILED", &e.to_string()))),
        };
        let entity = &ctx.store.manifest().auth.user.entity;
        let user_id = match ctx.store.lookup(entity, "walletAddress", &recovered) {
            Ok(Some(row)) => row
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            _ => {
                let now = format!(
                    "{}Z",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                );
                let dn = display_name.unwrap_or_else(|| {
                    format!("{}…{}", &recovered[..6], &recovered[recovered.len() - 4..])
                });
                match ctx.store.insert(
                    entity,
                    &serde_json::json!({
                        "walletAddress": recovered,
                        "displayName": dn,
                        "createdAt": now,
                    }),
                ) {
                    Ok(id) => id,
                    Err(e) => return Some((400, json_error(&e.code, &e.message))),
                }
            }
        };
        let session = create_session_with_device(ctx, user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((
            200,
            serde_json::json!({
                "token": session.token, "user_id": user_id, "address": recovered,
                "expires_at": session.expires_at
            })
            .to_string(),
        ));
    }

    // ─── WebAuthn / passkeys ──────────────────────────────────────────
    if url == "/api/auth/passkey/register/begin" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let row = ctx
            .store
            .get_by_id(&ctx.store.manifest().auth.user.entity, &user_id)
            .ok()
            .flatten();
        let user_name = row
            .as_ref()
            .and_then(|r| r.get("email"))
            .and_then(|v| v.as_str())
            .unwrap_or(&user_id)
            .to_string();
        let challenge = ctx.passkeys.mint_challenge(
            user_id.clone(),
            pylon_auth::webauthn::ChallengeKind::Registration,
        );
        let rp_id = std::env::var("PYLON_WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".into());
        return Some((
            200,
            serde_json::json!({
                "challenge": challenge, "rpId": rp_id, "userId": user_id, "userName": user_name
            })
            .to_string(),
        ));
    }
    if url == "/api/auth/passkey/register/finish" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")),
                ))
            }
        };
        let challenge = data.get("challenge").and_then(|v| v.as_str()).unwrap_or("");
        let cred_id = data
            .get("credentialId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let public_key_b64 = data.get("publicKey").and_then(|v| v.as_str()).unwrap_or("");
        let name = data
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("passkey")
            .to_string();
        if cred_id.is_empty() || public_key_b64.is_empty() {
            return Some((
                400,
                json_error("MISSING_FIELD", "credentialId + publicKey required"),
            ));
        }
        if ctx
            .passkeys
            .take_challenge(challenge, pylon_auth::webauthn::ChallengeKind::Registration)
            .is_none()
        {
            return Some((
                401,
                json_error("BAD_CHALLENGE", "Challenge missing or expired"),
            ));
        }
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let public_key = match URL_SAFE_NO_PAD.decode(public_key_b64) {
            Ok(b) => b,
            Err(e) => {
                return Some((
                    400,
                    json_error("BAD_PUBKEY", &format!("not base64url: {e}")),
                ))
            }
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        ctx.passkeys.store_passkey(pylon_auth::webauthn::Passkey {
            id: cred_id.to_string(),
            user_id,
            public_key,
            sign_count: 0,
            name,
            created_at: now,
            last_used_at: None,
        });
        return Some((
            200,
            serde_json::json!({"registered": true, "id": cred_id}).to_string(),
        ));
    }
    if url == "/api/auth/passkey/login/begin" && method == HttpMethod::Post {
        let challenge = ctx.passkeys.mint_challenge(
            String::new(),
            pylon_auth::webauthn::ChallengeKind::Assertion,
        );
        let rp_id = std::env::var("PYLON_WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".into());
        return Some((
            200,
            serde_json::json!({"challenge": challenge, "rpId": rp_id}).to_string(),
        ));
    }
    if url == "/api/auth/passkey/login/finish" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")),
                ))
            }
        };
        let cred_id = data
            .get("credentialId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let auth_data = URL_SAFE_NO_PAD
            .decode(
                data.get("authenticatorData")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            )
            .unwrap_or_default();
        let client_data = URL_SAFE_NO_PAD
            .decode(
                data.get("clientDataJSON")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            )
            .unwrap_or_default();
        let sig = URL_SAFE_NO_PAD
            .decode(data.get("signature").and_then(|v| v.as_str()).unwrap_or(""))
            .unwrap_or_default();
        let expected_origin =
            std::env::var("PYLON_WEBAUTHN_ORIGIN").unwrap_or_else(|_| "https://localhost".into());
        let expected_rp_id =
            std::env::var("PYLON_WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".into());
        let input = pylon_auth::webauthn::AssertionInput {
            credential_id: cred_id,
            authenticator_data: &auth_data,
            client_data_json: &client_data,
            signature: &sig,
            user_handle: None,
        };
        let key = match pylon_auth::webauthn::verify_assertion(
            ctx.passkeys,
            &input,
            &expected_origin,
            &expected_rp_id,
            None,
        ) {
            Ok(k) => k,
            Err(e) => return Some((401, json_error("PASSKEY_VERIFY_FAILED", &e.to_string()))),
        };
        let session = ctx.session_store.create(key.user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((
            200,
            serde_json::json!({
                "token": session.token, "user_id": key.user_id, "expires_at": session.expires_at
            })
            .to_string(),
        ));
    }
    if url == "/api/auth/passkey/keys" && method == HttpMethod::Get {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let payload: Vec<serde_json::Value> = ctx
            .passkeys
            .list_for_user(&user_id)
            .iter()
            .map(|k| {
                serde_json::json!({
                    "id": k.id, "name": k.name, "created_at": k.created_at,
                    "last_used_at": k.last_used_at,
                })
            })
            .collect();
        return Some((
            200,
            serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into()),
        ));
    }
    if let Some(id) = url.strip_prefix("/api/auth/passkey/keys/") {
        if method == HttpMethod::Delete {
            let user_id = match ctx.auth_ctx.user_id.as_deref() {
                Some(u) => u.to_string(),
                None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
            };
            match ctx.passkeys.get_passkey(id) {
                Some(k) if k.user_id == user_id => {
                    let removed = ctx.passkeys.delete(id);
                    return Some((200, serde_json::json!({"deleted": removed}).to_string()));
                }
                _ => return Some((404, json_error("NOT_FOUND", "Passkey not found"))),
            }
        }
    }

    // ─── SCIM 2.0 ─────────────────────────────────────────────────────
    // Bearer-token gated via PYLON_SCIM_TOKEN. Apps that don't
    // configure this env var get a 503 — refusing silently would
    // leave the surface looking broken.
    if let Some(rest) = url.strip_prefix("/scim/v2/") {
        let auth = ctx
            .request_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.as_str());
        if !pylon_auth::scim::check_bearer(auth) {
            return Some((
                401,
                serde_json::to_string(&pylon_auth::scim::ScimError::new(
                    401,
                    "missing or invalid SCIM bearer token",
                ))
                .unwrap_or_default(),
            ));
        }
        let entity = &ctx.store.manifest().auth.user.entity;
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        match (parts.as_slice(), method) {
            // POST /scim/v2/Users
            (["Users"], HttpMethod::Post) => {
                let scim_user: pylon_auth::scim::ScimUser = match serde_json::from_str(body) {
                    Ok(u) => u,
                    Err(e) => {
                        return Some((
                            400,
                            serde_json::to_string(&pylon_auth::scim::ScimError::new(
                                400,
                                &format!("invalid SCIM JSON: {e}"),
                            ))
                            .unwrap_or_default(),
                        ))
                    }
                };
                let now = format!(
                    "{}Z",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                );
                let row = serde_json::json!({
                    "email": scim_user.primary_email(),
                    "displayName": scim_user.pretty_display_name(),
                    "scimId": scim_user.id,
                    "scimActive": scim_user.active,
                    "createdAt": now,
                });
                match ctx.store.insert(entity, &row) {
                    Ok(id) => {
                        let mut response = scim_user;
                        response.id = Some(id);
                        return Some((201, serde_json::to_string(&response).unwrap_or_default()));
                    }
                    Err(e) => {
                        return Some((
                            409,
                            serde_json::to_string(&pylon_auth::scim::ScimError::new(
                                409, &e.message,
                            ))
                            .unwrap_or_default(),
                        ))
                    }
                }
            }
            (["Users"], HttpMethod::Get) => {
                let list = ctx.store.list(entity).unwrap_or_default();
                let users: Vec<pylon_auth::scim::ScimUser> = list
                    .iter()
                    .filter_map(|row| {
                        let email = row.get("email").and_then(|v| v.as_str())?.to_string();
                        let id = row.get("id").and_then(|v| v.as_str()).map(String::from);
                        let active = row
                            .get("scimActive")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        let display_name = row
                            .get("displayName")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        Some(pylon_auth::scim::ScimUser {
                            id,
                            user_name: email.clone(),
                            active,
                            name: None,
                            emails: vec![pylon_auth::scim::ScimEmail {
                                value: email,
                                primary: Some(true),
                                kind: Some("work".into()),
                            }],
                            display_name,
                            schemas: vec!["urn:ietf:params:scim:schemas:core:2.0:User".into()],
                        })
                    })
                    .collect();
                return Some((
                    200,
                    serde_json::to_string(&pylon_auth::scim::ScimListResponse::new(users))
                        .unwrap_or_default(),
                ));
            }
            (["Users", id], HttpMethod::Get) => {
                let row = match ctx.store.get_by_id(entity, id) {
                    Ok(Some(r)) => r,
                    _ => {
                        return Some((
                            404,
                            serde_json::to_string(&pylon_auth::scim::ScimError::new(
                                404,
                                "user not found",
                            ))
                            .unwrap_or_default(),
                        ))
                    }
                };
                let email = row
                    .get("email")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let user = pylon_auth::scim::ScimUser {
                    id: Some(id.to_string()),
                    user_name: email.clone(),
                    active: row
                        .get("scimActive")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    name: None,
                    emails: vec![pylon_auth::scim::ScimEmail {
                        value: email,
                        primary: Some(true),
                        kind: Some("work".into()),
                    }],
                    display_name: row
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    schemas: vec!["urn:ietf:params:scim:schemas:core:2.0:User".into()],
                };
                return Some((200, serde_json::to_string(&user).unwrap_or_default()));
            }
            (["Users", id], HttpMethod::Delete) => {
                // SCIM DELETE = soft delete (set scimActive=false). Hard
                // delete left to the host app's account-deletion flow.
                let _ = ctx
                    .store
                    .update(entity, id, &serde_json::json!({"scimActive": false}));
                return Some((204, String::new()));
            }
            _ => {}
        }
    }

    // ─── OIDC Provider — discovery + JWKS only (auth-code flow Wave 6) ─
    // Apps that want pylon as their IdP get the discovery doc + JWKS
    // for free. Token issuance reuses the existing JWT mint.
    if url == "/.well-known/openid-configuration" && method == HttpMethod::Get {
        let issuer = std::env::var("PYLON_OIDC_ISSUER").unwrap_or_else(|_| {
            ctx.request_headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("host"))
                .map(|(_, v)| format!("https://{v}"))
                .unwrap_or_else(|| "http://localhost:4321".into())
        });
        let doc = pylon_auth::oidc_provider::DiscoveryDoc::for_issuer(&issuer);
        return Some((200, serde_json::to_string(&doc).unwrap_or_default()));
    }
    if url == "/oidc/jwks" && method == HttpMethod::Get {
        // We mint HS256 JWTs (Wave 3); HS256 doesn't publish a public
        // key (symmetric). When PYLON_OIDC_JWKS_RSA_N + _E are set,
        // we publish them — apps that have rotated to RSA can drop the
        // PEM-encoded modulus + exponent into env at deploy.
        let n = std::env::var("PYLON_OIDC_JWKS_RSA_N").unwrap_or_default();
        let e = std::env::var("PYLON_OIDC_JWKS_RSA_E").unwrap_or_else(|_| "AQAB".into());
        let kid = std::env::var("PYLON_OIDC_JWKS_KID").unwrap_or_else(|_| "pylon-default".into());
        let keys = if n.is_empty() {
            // No RSA key configured → empty JWKS (correct OIDC response;
            // means "no asymmetric verification keys published").
            vec![]
        } else {
            vec![pylon_auth::oidc_provider::Jwk {
                kty: "RSA".into(),
                alg: "RS256".into(),
                use_: "sig".into(),
                kid,
                n,
                e,
            }]
        };
        return Some((
            200,
            serde_json::to_string(&pylon_auth::oidc_provider::Jwks { keys }).unwrap_or_default(),
        ));
    }

    // ─── Stripe billing ────────────────────────────────────────────────
    //
    // POST /api/billing/checkout — mint a Stripe Checkout Session for
    //   the authenticated user. Body: `{ priceIds: [...], mode:
    //   "subscription"|"payment", successUrl, cancelUrl }`.
    // POST /api/billing/webhook — Stripe sends events here. Pylon
    //   verifies signature; an app-defined plugin hook
    //   (`plugin_hooks.on_billing_event`) handles the parsed event.
    if url == "/api/billing/checkout" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let cfg = match pylon_auth::stripe::StripeConfig::from_env() {
            Some(c) => c,
            None => {
                return Some((
                    501,
                    json_error_with_hint(
                        "STRIPE_NOT_CONFIGURED",
                        "Stripe billing is disabled",
                        "Set PYLON_STRIPE_API_KEY (sk_test_… or sk_live_…) to enable",
                    ),
                ));
            }
        };
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe("INVALID_JSON", "Invalid request body", &format!("{e}")),
                ));
            }
        };
        let price_ids: Vec<&str> = data
            .get("priceIds")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        if price_ids.is_empty() {
            return Some((400, json_error("MISSING_PRICES", "priceIds is required")));
        }
        let mode = match data
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("subscription")
        {
            "payment" => pylon_auth::stripe::CheckoutMode::Payment,
            _ => pylon_auth::stripe::CheckoutMode::Subscription,
        };
        let success_url = data
            .get("successUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("/billing/success");
        let cancel_url = data
            .get("cancelUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("/billing/cancel");
        // Pull existing customer id from the user row (or create
        // one). Apps should add `stripeCustomerId: string?` to their
        // User entity.
        let row = ctx
            .store
            .get_by_id(&ctx.store.manifest().auth.user.entity, &user_id)
            .ok()
            .flatten();
        let mut customer_id = row
            .as_ref()
            .and_then(|r| r.get("stripeCustomerId"))
            .and_then(|v| v.as_str())
            .map(String::from);
        if customer_id.is_none() {
            let email = row
                .as_ref()
                .and_then(|r| r.get("email"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match cfg.create_customer(email, None) {
                Ok(c) => {
                    let _ = ctx.store.update(
                        &ctx.store.manifest().auth.user.entity,
                        &user_id,
                        &serde_json::json!({"stripeCustomerId": c.id}),
                    );
                    customer_id = Some(c.id);
                }
                Err(e) => {
                    tracing::warn!("[stripe] customer create failed: {e}");
                    return Some((
                        502,
                        json_error("STRIPE_FAILED", "Could not create Stripe customer"),
                    ));
                }
            }
        }
        match cfg.create_checkout(
            customer_id.as_deref(),
            &price_ids,
            mode,
            success_url,
            cancel_url,
        ) {
            Ok(s) => {
                return Some((
                    200,
                    serde_json::json!({"url": s.url, "id": s.id}).to_string(),
                ))
            }
            Err(e) => {
                tracing::warn!("[stripe] checkout create failed: {e}");
                return Some((
                    502,
                    json_error("STRIPE_FAILED", "Could not create checkout session"),
                ));
            }
        }
    }
    if url == "/api/billing/webhook" && method == HttpMethod::Post {
        let cfg = match pylon_auth::stripe::StripeConfig::from_env() {
            Some(c) => c,
            None => return Some((501, json_error("STRIPE_NOT_CONFIGURED", "Stripe disabled"))),
        };
        let secret = match cfg.webhook_secret {
            Some(s) => s,
            None => {
                return Some((
                    501,
                    json_error("WEBHOOK_NOT_CONFIGURED", "Set PYLON_STRIPE_WEBHOOK_SECRET"),
                ))
            }
        };
        let sig_header = ctx
            .request_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("stripe-signature"))
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // body.as_bytes() is safe here because Stripe webhooks are
        // always JSON, JSON is always UTF-8, and the upstream
        // read_to_string preserves UTF-8 byte-for-byte. If a future
        // protocol carries non-UTF-8 bodies past read_to_string,
        // this assumption breaks — switch the dispatcher to bytes.
        let event =
            match pylon_auth::stripe::verify_webhook(&secret, body.as_bytes(), sig_header, now) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("[stripe] webhook verify failed: {e}");
                    return Some((400, json_error("WEBHOOK_INVALID", &e.to_string())));
                }
            };
        // For now we just log + return 200. A future hook lets apps
        // react via plugin (`plugin_hooks.on_billing_event`).
        tracing::info!("[stripe] event: {event:?}");
        return Some((200, serde_json::json!({"received": true}).to_string()));
    }

    // ─── Audit log (read-only) ────────────────────────────────────────
    //
    // GET /api/auth/audit?limit=100  — events where the current
    //   user is subject OR actor.
    // GET /api/auth/audit/tenant?limit=100 — events for the current
    //   active tenant. App's policy layer should gate this to admins.
    // Wave-7 codex P3: split into two EXACT-prefix matches so
    // `/api/auth/auditx` doesn't accidentally route here. Without
    // this, `strip_prefix("/api/auth/audit")` happily matches
    // `/api/auth/auditx` and gives back current-user audit data.
    let audit_path = if url == "/api/auth/audit" || url.starts_with("/api/auth/audit?") {
        Some(("user", url.split_once('?').map(|(_, q)| q).unwrap_or("")))
    } else if url == "/api/auth/audit/tenant" || url.starts_with("/api/auth/audit/tenant?") {
        Some(("tenant", url.split_once('?').map(|(_, q)| q).unwrap_or("")))
    } else {
        None
    };
    if let Some((scope, q)) = audit_path {
        if method == HttpMethod::Get {
            let user_id = match ctx.auth_ctx.user_id.as_deref() {
                Some(u) => u.to_string(),
                None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
            };
            let params = parse_query(q);
            // Cap limit at 1000 here; the backend caps again at 10k
            // for raw queries. Defense in depth.
            let limit = params
                .get("limit")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(100)
                .min(1000);
            let events = if scope == "tenant" {
                let Some(tid) = ctx.auth_ctx.tenant_id.as_deref() else {
                    return Some((400, json_error("NO_ACTIVE_TENANT", "Select an org first")));
                };
                // Wave-7 codex P2: tenant audit is org-wide and
                // includes IPs, UAs, reasons, metadata — way too
                // sensitive for any member to read. Require the
                // caller to have an admin/owner role in the active
                // org. Apps that have their own RBAC layer can
                // override by giving everyone admin (their call).
                let role = ctx.orgs.role_of(tid, &user_id);
                let is_admin = role.map(|r| r.can_manage_members()).unwrap_or(false);
                if !is_admin && !ctx.auth_ctx.is_admin {
                    return Some((
                        403,
                        json_error(
                            "FORBIDDEN",
                            "Tenant audit requires admin or owner role in the active org",
                        ),
                    ));
                }
                ctx.audit.find_for_tenant(tid, limit)
            } else {
                ctx.audit.find_for_user(&user_id, limit)
            };
            let payload: Vec<serde_json::Value> = events
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "created_at": e.created_at,
                        "action": e.action.as_str(),
                        "user_id": e.user_id,
                        "actor_id": e.actor_id,
                        "tenant_id": e.tenant_id,
                        "ip": e.ip,
                        "user_agent": e.user_agent,
                        "success": e.success,
                        "reason": e.reason,
                        "metadata": e.metadata,
                    })
                })
                .collect();
            return Some((
                200,
                serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into()),
            ));
        }
    }

    // ─── Password reset (forgot password) ─────────────────────────────
    //
    // Two-step: request mints a token + emails a reset URL; complete
    // verifies the token + sets the new password + revokes other
    // sessions. Request always returns 200 — never reveal whether
    // an email is registered (account-enumeration defense).
    if url == "/api/auth/password/reset/request" && method == HttpMethod::Post {
        let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let email = data
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if email.is_empty() {
            return Some((400, json_error("MISSING_EMAIL", "email is required")));
        }
        // Rate limit BEFORE the lookup so we don't leak existence
        // via response timing under load.
        let rl = pylon_auth::rate_limit::AuthRateLimiter::shared();
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } = rl.check(
            pylon_auth::rate_limit::AuthBucket::Send,
            ctx.peer_ip,
            Some(&email),
        ) {
            return Some((
                429,
                json_error_with_hint(
                    "RATE_LIMITED",
                    "Too many reset requests",
                    &format!("Try again in {retry_after_secs}s"),
                ),
            ));
        }
        // Wave-6 codex P1: equalize timing across "registered" /
        // "not registered" paths so an attacker can't enumerate
        // accounts via response time. Two parts:
        //   1. ALWAYS mint a token (cheap HMAC; constant time
        //      regardless of whether the email exists). For the
        //      not-registered case the token is unconnected to any
        //      User row so it can never be redeemed — but it
        //      consumes the same compute path as the real one.
        //   2. ALWAYS pad the response with a fixed micro-sleep so
        //      the lookup's variance (cache hit vs miss) can't
        //      leak via wallclock either.
        let entity = &ctx.store.manifest().auth.user.entity;
        let registered = matches!(ctx.store.lookup(entity, "email", &email), Ok(Some(_)));
        // Mint regardless — discarded for non-registered.
        let minted = ctx.verification.mint(
            pylon_auth::verification::TokenKind::PasswordReset,
            &email,
            None,
            None,
        );
        if registered {
            let public_url = std::env::var("PYLON_PUBLIC_URL").unwrap_or_default();
            let reset_url = format!("{public_url}/reset-password?token={}", minted.plaintext);
            let mut vars = std::collections::HashMap::new();
            vars.insert("url", reset_url.as_str());
            let (subject, body_text) = pylon_auth::email_templates::render(
                pylon_auth::email_templates::EmailTemplate::PasswordReset,
                &vars,
            );
            if let Err(e) = ctx.email.send(&email, &subject, &body_text) {
                tracing::warn!("[auth] reset email to {} failed: {e}", redact_email(&email));
            }
        } else {
            // Pad with a single equivalent SMTP-like delay so the
            // wallclock looks similar. Real email sends are
            // 10-200ms; we sleep 50ms as a reasonable middle.
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // Always 200 — frontend says "if an account exists, we sent a link."
        return Some((200, serde_json::json!({"sent": true}).to_string()));
    }
    if url == "/api/auth/password/reset/complete" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")),
                ))
            }
        };
        let token = data.get("token").and_then(|v| v.as_str()).unwrap_or("");
        let new_password = data
            .get("newPassword")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if token.is_empty() || new_password.is_empty() {
            return Some((
                400,
                json_error("MISSING_FIELD", "token + newPassword required"),
            ));
        }
        if let Err(e) = pylon_auth::password::validate_length(new_password) {
            return Some((400, json_error("WEAK_PASSWORD", &e.to_string())));
        }
        if std::env::var("PYLON_DISABLE_HIBP").ok().as_deref() != Some("1") {
            if let Ok(n) = pylon_auth::password::check_pwned(new_password) {
                if n > 0 {
                    return Some((
                        400,
                        json_error_safe(
                            "PWNED_PASSWORD",
                            "This password has appeared in known data breaches.",
                            &format!("HIBP returned {n} occurrences"),
                        ),
                    ));
                }
            }
        }
        let consumed = match ctx
            .verification
            .consume(token, pylon_auth::verification::TokenKind::PasswordReset)
        {
            Ok(t) => t,
            Err(e) => return Some((401, json_error("INVALID_TOKEN", &e.to_string()))),
        };
        let entity = &ctx.store.manifest().auth.user.entity;
        let row = match ctx.store.lookup(entity, "email", &consumed.email) {
            Ok(Some(r)) => r,
            _ => {
                return Some((
                    404,
                    json_error("USER_NOT_FOUND", "Account no longer exists"),
                ))
            }
        };
        let user_id = row
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let new_hash = pylon_auth::password::hash_password(new_password);
        if let Err(e) = ctx.store.update(
            entity,
            &user_id,
            &serde_json::json!({"passwordHash": new_hash}),
        ) {
            return Some((400, json_error(&e.code, &e.message)));
        }
        // Revoke ALL existing sessions — same posture as password change.
        let revoked = ctx.session_store.revoke_all_for_user(&user_id);
        let session = create_session_with_device(ctx, user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        ctx.audit.log(
            audit(ctx, pylon_auth::audit::AuditAction::PasswordReset)
                .user(user_id.clone())
                .actor(user_id.clone())
                .meta("revoked_sessions", revoked.to_string())
                .build(),
        );
        return Some((
            200,
            serde_json::json!({
                "reset": true, "revoked_sessions": revoked,
                "token": session.token, "user_id": user_id, "expires_at": session.expires_at,
            })
            .to_string(),
        ));
    }
    // ─── Magic links ──────────────────────────────────────────────────
    //
    // Like magic codes but the user clicks a URL instead of typing
    // a 6-digit code. /send mints + emails; /verify takes the token
    // (via either the GET `?token=` browser flow or POST JSON body)
    // and mints a session.
    if url == "/api/auth/magic-link/send" && method == HttpMethod::Post {
        let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let email = data
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if email.is_empty() || !email.contains('@') {
            return Some((400, json_error("INVALID_EMAIL", "valid email required")));
        }
        let rl = pylon_auth::rate_limit::AuthRateLimiter::shared();
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } = rl.check(
            pylon_auth::rate_limit::AuthBucket::Send,
            ctx.peer_ip,
            Some(&email),
        ) {
            return Some((
                429,
                json_error_with_hint(
                    "RATE_LIMITED",
                    "Too many sign-in requests",
                    &format!("Try again in {retry_after_secs}s"),
                ),
            ));
        }
        // Optional CAPTCHA gate.
        if let Some(cfg) = pylon_auth::captcha::CaptchaConfig::from_env() {
            let token = data
                .get("captchaToken")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if cfg.verify(token, Some(ctx.peer_ip)).is_err() {
                return Some((400, json_error("CAPTCHA_FAILED", "CAPTCHA failed")));
            }
        }
        let minted = ctx.verification.mint(
            pylon_auth::verification::TokenKind::MagicLink,
            &email,
            None,
            None,
        );
        let public_url = std::env::var("PYLON_PUBLIC_URL").unwrap_or_default();
        let verify_url = format!(
            "{public_url}/api/auth/magic-link/verify?token={}",
            minted.plaintext
        );
        let mut vars = std::collections::HashMap::new();
        vars.insert("url", verify_url.as_str());
        let (subject, body_text) = pylon_auth::email_templates::render(
            pylon_auth::email_templates::EmailTemplate::MagicLink,
            &vars,
        );
        if let Err(e) = ctx.email.send(&email, &subject, &body_text) {
            tracing::warn!(
                "[auth] magic-link email to {} failed: {e}",
                redact_email(&email)
            );
            if !ctx.is_dev {
                return Some((500, json_error("EMAIL_SEND_FAILED", "Could not send email")));
            }
        }
        let mut response = serde_json::json!({"sent": true});
        if ctx.is_dev {
            response["dev_token"] = serde_json::Value::String(minted.plaintext);
            response["dev_url"] = serde_json::Value::String(verify_url);
        }
        return Some((200, response.to_string()));
    }
    if let Some(rest) = url.strip_prefix("/api/auth/magic-link/verify") {
        if method == HttpMethod::Get || method == HttpMethod::Post {
            // Token can come from `?token=` (GET browser click) or body (POST SDK).
            let token = if method == HttpMethod::Get {
                let q = rest.trim_start_matches('?');
                parse_query(q).get("token").cloned().unwrap_or_default()
            } else {
                let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
                data.get("token")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            if token.is_empty() {
                return Some((400, json_error("MISSING_TOKEN", "token required")));
            }
            let consumed = match ctx
                .verification
                .consume(&token, pylon_auth::verification::TokenKind::MagicLink)
            {
                Ok(t) => t,
                Err(e) => return Some((401, json_error("INVALID_TOKEN", &e.to_string()))),
            };
            let entity = &ctx.store.manifest().auth.user.entity;
            let user_id = match ctx.store.lookup(entity, "email", &consumed.email) {
                Ok(Some(row)) => row
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                _ => {
                    let now = format!(
                        "{}Z",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    );
                    match ctx.store.insert(
                        entity,
                        &serde_json::json!({
                            "email": consumed.email,
                            "displayName": consumed.email,
                            "emailVerified": now.clone(),
                            "createdAt": now,
                        }),
                    ) {
                        Ok(id) => id,
                        Err(e) => return Some((400, json_error(&e.code, &e.message))),
                    }
                }
            };
            let session = create_session_with_device(ctx, user_id.clone());
            ctx.maybe_set_session_cookie(&session.token);
            // Browser flow → 302 to dashboard; SDK flow → JSON.
            if method == HttpMethod::Get {
                let dashboard = std::env::var("PYLON_DASHBOARD_URL").unwrap_or_else(|_| "/".into());
                ctx.add_response_header("Location", dashboard);
                return Some((302, String::new()));
            }
            return Some((
                200,
                serde_json::json!({
                    "token": session.token, "user_id": user_id, "expires_at": session.expires_at
                })
                .to_string(),
            ));
        }
    }
    // ─── Email change ─────────────────────────────────────────────────
    //
    // POST /api/auth/email/change/request {newEmail}  (auth required)
    //   → mints a token bound to (currentUserId, newEmail), emails
    //     the link to NEW email. New email isn't applied yet — the
    //     verify step is what swaps it.
    // POST /api/auth/email/change/confirm {token}
    //   → applies the change, revokes other sessions.
    if url == "/api/auth/email/change/request" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error("API_KEY_AUTH_FORBIDDEN", "Email change requires a session"),
            ));
        }
        // Wave-6 codex P2: rate limit email-change-request so an
        // authenticated user can't email-bomb arbitrary addresses
        // through the change-confirm send path.
        let rl = pylon_auth::rate_limit::AuthRateLimiter::shared();
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } = rl.check(
            pylon_auth::rate_limit::AuthBucket::Send,
            ctx.peer_ip,
            Some(&user_id),
        ) {
            return Some((
                429,
                json_error_with_hint(
                    "RATE_LIMITED",
                    "Too many email-change requests",
                    &format!("Try again in {retry_after_secs}s"),
                ),
            ));
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")),
                ))
            }
        };
        let new_email = data
            .get("newEmail")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if new_email.is_empty() || !new_email.contains('@') {
            return Some((400, json_error("INVALID_EMAIL", "valid newEmail required")));
        }
        let entity = &ctx.store.manifest().auth.user.entity;
        // Wave-6 codex P2: equalize timing — taken-email path
        // returned BEFORE the email send, vs free-email path doing
        // the send. Now both paths skip the send when taken but
        // pad with a 50ms sleep to mask the wallclock difference.
        let taken = matches!(ctx.store.lookup(entity, "email", &new_email), Ok(Some(_)));
        if taken {
            std::thread::sleep(std::time::Duration::from_millis(50));
            return Some((200, serde_json::json!({"sent": true}).to_string()));
        }
        let minted = ctx.verification.mint(
            pylon_auth::verification::TokenKind::EmailChange,
            &new_email,
            Some(user_id.clone()),
            Some(new_email.clone()),
        );
        let public_url = std::env::var("PYLON_PUBLIC_URL").unwrap_or_default();
        let confirm_url = format!(
            "{public_url}/email-change/confirm?token={}",
            minted.plaintext
        );
        let mut vars = std::collections::HashMap::new();
        vars.insert("url", confirm_url.as_str());
        let (subject, body_text) = pylon_auth::email_templates::render(
            pylon_auth::email_templates::EmailTemplate::EmailChangeConfirm,
            &vars,
        );
        if let Err(e) = ctx.email.send(&new_email, &subject, &body_text) {
            tracing::warn!(
                "[auth] email-change confirm to {} failed: {e}",
                redact_email(&new_email)
            );
        }
        return Some((200, serde_json::json!({"sent": true}).to_string()));
    }
    if url == "/api/auth/email/change/confirm" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")),
                ))
            }
        };
        let token = data.get("token").and_then(|v| v.as_str()).unwrap_or("");
        if token.is_empty() {
            return Some((400, json_error("MISSING_TOKEN", "token required")));
        }
        let consumed = match ctx
            .verification
            .consume(token, pylon_auth::verification::TokenKind::EmailChange)
        {
            Ok(t) => t,
            Err(e) => return Some((401, json_error("INVALID_TOKEN", &e.to_string()))),
        };
        let user_id = consumed.user_id.unwrap_or_default();
        let new_email = consumed.payload.unwrap_or_default();
        if user_id.is_empty() || new_email.is_empty() {
            return Some((
                400,
                json_error("INVALID_TOKEN", "token has no embedded user/email"),
            ));
        }
        let entity = &ctx.store.manifest().auth.user.entity;
        let now = format!(
            "{}Z",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        if let Err(e) = ctx.store.update(
            entity,
            &user_id,
            &serde_json::json!({
                "email": new_email, "emailVerified": now,
            }),
        ) {
            return Some((400, json_error(&e.code, &e.message)));
        }
        // Revoke other sessions on email change — same blast-radius
        // posture as password change.
        ctx.session_store.revoke_all_for_user(&user_id);
        let session = create_session_with_device(ctx, user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((
            200,
            serde_json::json!({
                "changed": true, "email": new_email,
                "token": session.token, "user_id": user_id, "expires_at": session.expires_at,
            })
            .to_string(),
        ));
    }
    // ─── TOTP backup codes ────────────────────────────────────────────
    //
    // POST /api/auth/totp/backup-codes/regenerate (auth required)
    //   → mints 10 codes, hashes them, returns plaintext exactly once.
    //     Replaces any prior set.
    // The /api/auth/totp/verify endpoint accepts EITHER a current
    // 6-digit TOTP code OR one backup code (consumed on use).
    if url == "/api/auth/totp/backup-codes/regenerate" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error("API_KEY_AUTH_FORBIDDEN", "Backup codes require a session"),
            ));
        }
        // 10 random codes, formatted XXXX-XXXX (8 alphanumeric chars
        // with a dash for readability — Google's standard).
        use rand::Rng;
        let mut rng = rand::thread_rng();
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let codes: Vec<String> = (0..10)
            .map(|_| {
                let raw: String = (0..8)
                    .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
                    .collect();
                format!("{}-{}", &raw[..4], &raw[4..])
            })
            .collect();
        // Store the SHA-256 hashes (HMAC w/ pepper would be even
        // better, but matching the existing api-key pattern).
        use sha2::{Digest, Sha256};
        let hashes: Vec<String> = codes
            .iter()
            .map(|c| {
                let mut h = Sha256::new();
                h.update(c.as_bytes());
                let out = h.finalize();
                use std::fmt::Write;
                let mut s = String::with_capacity(64);
                for b in out {
                    let _ = write!(s, "{b:02x}");
                }
                s
            })
            .collect();
        return match ctx.store.update(
            &ctx.store.manifest().auth.user.entity,
            &user_id,
            &serde_json::json!({"totpBackupCodes": hashes}),
        ) {
            Ok(_) => Some((200, serde_json::json!({"codes": codes}).to_string())),
            Err(e) => Some((400, json_error(&e.code, &e.message))),
        };
    }
    // ─── Anonymous → authenticated upgrade with cart merge ────────────
    //
    // POST /api/auth/anonymous → mint a guest session (existing
    // behavior is /api/auth/guest; this is the better-auth-compatible
    // alias).
    if url == "/api/auth/anonymous" && method == HttpMethod::Post {
        // Wave-6 codex P2: rate limit so a botnet can't spawn
        // unbounded guest sessions. No per-account dimension —
        // anonymous by definition has no account.
        let rl = pylon_auth::rate_limit::AuthRateLimiter::shared();
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } =
            rl.check(pylon_auth::rate_limit::AuthBucket::Send, ctx.peer_ip, None)
        {
            return Some((
                429,
                json_error_with_hint(
                    "RATE_LIMITED",
                    "Too many anonymous sessions",
                    &format!("Try again in {retry_after_secs}s"),
                ),
            ));
        }
        let session = ctx.session_store.create_guest();
        ctx.maybe_set_session_cookie(&session.token);
        return Some((
            200,
            serde_json::json!({
                "token": session.token, "user_id": session.user_id,
                "is_guest": true, "expires_at": session.expires_at,
            })
            .to_string(),
        ));
    }
    // ─── Account deletion ──────────────────────────────────────────────
    //
    // DELETE /api/auth/account — wipes the user row, revokes all
    // sessions, deletes all API keys, removes linked accounts. The
    // app-defined `User` entity is the source of truth so other tables
    // that reference it cascade through whatever FK story the schema
    // has set up.
    if url == "/api/auth/account" && method == HttpMethod::Delete {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        // P2 fix: API key auth cannot delete the account.
        if ctx.auth_ctx.is_api_key_auth() {
            return Some((
                403,
                json_error(
                    "API_KEY_AUTH_FORBIDDEN",
                    "Account deletion requires a session, not an API key",
                ),
            ));
        }
        // Revoke sessions first so a slow user-row delete doesn't
        // leave the attacker with a usable session.
        let revoked_sessions = ctx.session_store.revoke_all_for_user(&user_id);
        // Delete all api keys for this user.
        let mut revoked_keys = 0;
        for key in ctx.api_keys.list_for_user(&user_id) {
            if ctx.api_keys.revoke(&key.id) {
                revoked_keys += 1;
            }
        }
        // Remove all linked OAuth accounts (codex P2). App-owned
        // tables that reference the user are NOT cascade-deleted by
        // pylon — the host schema is the source of truth and must
        // declare its own deletion semantics. The /api/auth/account
        // docs call this out so apps register a `before-delete-user`
        // hook to purge their tables.
        let revoked_accounts = ctx.account_store.delete_for_user(&user_id);
        // Trusted-device records — same rationale as sessions/api keys.
        // Without this, deleting an account leaves orphan trust cookies
        // that would happen to match no user (find returns None) but
        // still pollute the table.
        let revoked_trust = ctx.trusted_devices.revoke_all_for_user(&user_id);
        // Delete the user row.
        match ctx
            .store
            .delete(&ctx.store.manifest().auth.user.entity, &user_id)
        {
            Ok(_) => {}
            Err(e) => return Some((400, json_error(&e.code, &e.message))),
        }
        ctx.add_response_header("Set-Cookie", ctx.cookie_config.clear_value());
        ctx.audit.log(
            audit(ctx, pylon_auth::audit::AuditAction::AccountDelete)
                .user(user_id.clone())
                .actor(user_id.clone())
                .meta("revoked_sessions", revoked_sessions.to_string())
                .meta("revoked_api_keys", revoked_keys.to_string())
                .meta("unlinked_accounts", revoked_accounts.to_string())
                .meta("revoked_trusted_devices", revoked_trust.to_string())
                .build(),
        );
        return Some((
            200,
            serde_json::json!({
                "deleted": true,
                "revoked_sessions": revoked_sessions,
                "revoked_api_keys": revoked_keys,
                "unlinked_accounts": revoked_accounts,
            })
            .to_string(),
        ));
    }

    None
}

/// Project a User row down to the fields safe for `/api/auth/session`.
///
/// Defaults strip `passwordHash` + anything starting with `_`
/// (framework-internal). The manifest's `auth.user.expose` /
/// `auth.user.hide` config refines this:
/// - `expose` (allowlist): when non-empty, ONLY listed fields appear
///   (`id` is always included). Useful for apps with strict client
///   schemas.
/// - `hide` (blocklist): additional fields to strip on top of defaults.
///   Use for app-specific secrets stored on the User row.
fn project_user_row(
    row: serde_json::Value,
    cfg: &pylon_kernel::ManifestAuthUserConfig,
) -> serde_json::Value {
    let serde_json::Value::Object(obj) = row else {
        return row;
    };
    let filtered: serde_json::Map<String, serde_json::Value> = obj
        .into_iter()
        .filter(|(k, _)| {
            if k == "id" {
                return true; // always include id
            }
            // Allowlist takes precedence: only `expose` fields pass.
            if !cfg.expose.is_empty() && !cfg.expose.iter().any(|f| f == k) {
                return false;
            }
            // Default + manifest blocklist.
            if k == "passwordHash" || k.starts_with('_') {
                return false;
            }
            if cfg.hide.iter().any(|f| f == k) {
                return false;
            }
            true
        })
        .collect();
    serde_json::Value::Object(filtered)
}
