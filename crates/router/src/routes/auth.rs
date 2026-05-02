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
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } =
            rl.check(pylon_auth::rate_limit::AuthBucket::Send, ctx.peer_ip, Some(&email))
        {
            return Some((
                429,
                json_error_with_hint("RATE_LIMITED", "Too many sign-in requests",
                    &format!("Try again in {retry_after_secs}s")),
            ));
        }
        // Optional CAPTCHA gate. When PYLON_CAPTCHA_PROVIDER+SECRET
        // are set, the request must include `captchaToken`. Skipped
        // entirely when unconfigured so existing apps keep working.
        if let Some(cfg) = pylon_auth::captcha::CaptchaConfig::from_env() {
            let token = data.get("captchaToken").and_then(|v| v.as_str()).unwrap_or("");
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
        let subject = "Your sign-in code";
        let body_text =
            format!("Your sign-in code is: {code}\n\nThis code will expire in 10 minutes.");
        if let Err(e) = ctx.email.send(&email, subject, &body_text) {
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
                let session = ctx.session_store.create(user_id.clone());
                ctx.maybe_set_session_cookie(&session.token);
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
            let token = data.get("captchaToken").and_then(|v| v.as_str()).unwrap_or("");
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

        let session = ctx.session_store.create(user_id.clone());
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
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } =
            rl.check(pylon_auth::rate_limit::AuthBucket::Login, ctx.peer_ip, Some(&email))
        {
            return Some((
                429,
                json_error_with_hint("RATE_LIMITED", "Too many login attempts",
                    &format!("Try again in {retry_after_secs}s")),
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
        let session = ctx.session_store.create(user_id.clone());
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
            a.get("provider").and_then(|v| v.as_str()).unwrap_or("")
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
                        json_error("OAUTH_PROVIDER_BROKEN", &format!("provider {provider} misconfigured: {e}")),
                    ));
                }
            };
            let state = ctx
                .oauth_state
                .create_with_pkce(provider, &callback, &error_callback, pkce_verifier);
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
            let is_form_post = ctx
                .request_headers
                .iter()
                .any(|(k, v)| {
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
                    Ok((_user_id, session)) => {
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
                Ok((_user_id, session)) => {
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
            let expires_at = data
                .get("expires_at")
                .and_then(|v| v.as_u64());
            let (plaintext, key) =
                ctx.api_keys.create(user_id.to_string(), name, scopes, expires_at);
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
            return Some((200, serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into())));
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
            match ctx.api_keys.list_for_user(user_id).iter().find(|k| k.id == id) {
                Some(_) => {
                    let revoked = ctx.api_keys.revoke(id);
                    return Some((
                        200,
                        serde_json::json!({"revoked": revoked}).to_string(),
                    ));
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
        let row = match ctx.store.get_by_id(
            &ctx.store.manifest().auth.user.entity,
            &user_id,
        ) {
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
        let session = ctx.session_store.create(user_id.clone());
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
                json_error("API_KEY_AUTH_FORBIDDEN", "TOTP enrollment requires a session"),
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
            let stored_blob = row
                .get("totpSecret")
                .and_then(|v| v.as_str())
                .unwrap_or("");
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
            Err(_) => return Some((500, json_error("TOTP_BAD_SECRET", "Stored secret is corrupt or PYLON_TOTP_ENCRYPTION_KEY missing"))),
        };
        let secret = match pylon_auth::totp::base32_decode(&secret_b32) {
            Ok(s) => s,
            Err(_) => return Some((500, json_error("TOTP_BAD_SECRET", "Stored secret is corrupt"))),
        };
        // Per-account rate limit on verify so backup-code brute
        // force can't churn through 10 codes in a second.
        let rl = pylon_auth::rate_limit::AuthRateLimiter::shared();
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } =
            rl.check(pylon_auth::rate_limit::AuthBucket::Login, ctx.peer_ip, Some(&user_id))
        {
            return Some((
                429,
                json_error_with_hint("RATE_LIMITED", "Too many TOTP attempts",
                    &format!("Try again in {retry_after_secs}s")),
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
                for b in candidate { let _ = write!(hex, "{b:02x}"); }
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
        if let Some(idx) = backup_consumed {
            if let Some(stored) = row.get("totpBackupCodes").and_then(|v| v.as_array()) {
                let kept: Vec<&serde_json::Value> = stored.iter().enumerate()
                    .filter(|(i, _)| *i != idx).map(|(_, v)| v).collect();
                let _ = ctx.store.update(
                    &ctx.store.manifest().auth.user.entity, &user_id,
                    &serde_json::json!({"totpBackupCodes": kept}),
                );
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
        return Some((
            200,
            serde_json::json!({"verified": true, "enrolled": !was_verified}).to_string(),
        ));
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
                let secret_b32 =
                    pylon_auth::totp::unseal_secret(secret_blob).unwrap_or_default();
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
            return Some((200, serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into())));
        }
    }

    if let Some(rest) = url.strip_prefix("/api/auth/orgs/") {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
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
                    return Some((403, json_error("FORBIDDEN", "Only owners can delete an org")));
                }
                let removed = ctx.orgs.delete(org_id);
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
                return Some((200, serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into())));
            }
            // /api/auth/orgs/:id/members/:user_id
            [_id, "members", target_user] if method == HttpMethod::Put => {
                if !caller_role.can_manage_members() {
                    return Some((403, json_error("FORBIDDEN", "Insufficient role")));
                }
                let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
                let role_str = data
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let role = match pylon_auth::org::OrgRole::from_str(role_str) {
                    Some(r) => r,
                    None => return Some((400, json_error("BAD_ROLE", "role must be owner|admin|member"))),
                };
                let updated = ctx.orgs.set_role(org_id, target_user, role);
                if !updated {
                    return Some((404, json_error("NOT_A_MEMBER", "Target user is not a member")));
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
                            json_error_safe("INVALID_JSON", "Invalid request body", &format!("{e}")),
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
                let subject = format!("You've been invited to {}", org.name);
                let body_text = format!(
                    "You've been invited to join {} on Pylon.\n\nAccept here: {}\n\nThis link expires in 7 days.",
                    org.name, accept_url
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
                return Some((200, serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into())));
            }
            [_id, "invites", invite_id] if method == HttpMethod::Delete => {
                if !caller_role.can_manage_members() {
                    return Some((403, json_error("FORBIDDEN", "Insufficient role")));
                }
                let revoked = ctx.orgs.revoke_invite(invite_id);
                return Some((200, serde_json::json!({"revoked": revoked}).to_string()));
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
                    None => return Some((401, json_error("AUTH_REQUIRED", "Login required to accept an invite"))),
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
            let token = data.get("captchaToken").and_then(|v| v.as_str()).unwrap_or("");
            if cfg.verify(token, Some(ctx.peer_ip)).is_err() {
                return Some((400, json_error("CAPTCHA_FAILED", "CAPTCHA failed")));
            }
        }
        let code = match ctx.phone_codes.try_create(phone) {
            Ok(c) => c,
            Err(pylon_auth::phone::PhoneCodeError::Throttled { retry_after_secs }) => {
                return Some((
                    429,
                    json_error_with_hint("RATE_LIMITED", "Code requested too recently",
                        &format!("Try again in {retry_after_secs}s")),
                ));
            }
            Err(pylon_auth::phone::PhoneCodeError::InvalidPhone) => {
                return Some((400, json_error("INVALID_PHONE", "Phone must be E.164 (+15551234567)")));
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
            Err(e) => return Some((400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("{e}")))),
        };
        let phone = data.get("phone").and_then(|v| v.as_str()).unwrap_or("");
        let code = data.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let display_name = data.get("displayName").and_then(|v| v.as_str()).map(String::from);
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
            Ok(Some(row)) => row.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            _ => {
                let now = format!("{}Z", std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
                let dn = display_name.unwrap_or_else(|| normalized.clone());
                match ctx.store.insert(entity, &serde_json::json!({
                    "phone": normalized,
                    "displayName": dn,
                    "phoneVerified": now.clone(),
                    "createdAt": now,
                })) {
                    Ok(id) => id,
                    Err(e) => return Some((400, json_error(&e.code, &e.message))),
                }
            }
        };
        let session = ctx.session_store.create(user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((200, serde_json::json!({
            "token": session.token, "user_id": user_id, "expires_at": session.expires_at
        }).to_string()));
    }

    // ─── SIWE — Sign-In With Ethereum ─────────────────────────────────
    if let Some(rest) = url.strip_prefix("/api/auth/siwe/nonce") {
        if method == HttpMethod::Get {
            let q = rest.trim_start_matches('?');
            let params = parse_query(q);
            let addr = params.get("address").map(|s| s.as_str()).unwrap_or("");
            if !addr.starts_with("0x") || addr.len() != 42 {
                return Some((400, json_error("INVALID_ADDRESS", "address must be 0x + 40 hex chars")));
            }
            let nonce = ctx.siwe.issue(addr);
            return Some((200, serde_json::json!({"nonce": nonce}).to_string()));
        }
    }
    if url == "/api/auth/siwe/verify" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return Some((400, json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")))),
        };
        let message_text = data.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let sig_hex = data.get("signature").and_then(|v| v.as_str()).unwrap_or("");
        let display_name = data.get("displayName").and_then(|v| v.as_str()).map(String::from);
        let parsed = match pylon_auth::siwe::parse_message(message_text) {
            Ok(m) => m,
            Err(e) => return Some((400, json_error("SIWE_BAD_MESSAGE", &e.to_string()))),
        };
        let expected_domain = ctx.request_headers.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("host"))
            .map(|(_, v)| v.split(':').next().unwrap_or(v).to_string())
            .unwrap_or_else(|| parsed.domain.clone());
        let recovered = match pylon_auth::siwe::verify(ctx.siwe, &parsed, sig_hex, &expected_domain) {
            Ok(addr) => addr,
            Err(e) => return Some((401, json_error("SIWE_VERIFY_FAILED", &e.to_string()))),
        };
        let entity = &ctx.store.manifest().auth.user.entity;
        let user_id = match ctx.store.lookup(entity, "walletAddress", &recovered) {
            Ok(Some(row)) => row.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            _ => {
                let now = format!("{}Z", std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
                let dn = display_name.unwrap_or_else(|| {
                    format!("{}…{}", &recovered[..6], &recovered[recovered.len() - 4..])
                });
                match ctx.store.insert(entity, &serde_json::json!({
                    "walletAddress": recovered,
                    "displayName": dn,
                    "createdAt": now,
                })) {
                    Ok(id) => id,
                    Err(e) => return Some((400, json_error(&e.code, &e.message))),
                }
            }
        };
        let session = ctx.session_store.create(user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((200, serde_json::json!({
            "token": session.token, "user_id": user_id, "address": recovered,
            "expires_at": session.expires_at
        }).to_string()));
    }

    // ─── WebAuthn / passkeys ──────────────────────────────────────────
    if url == "/api/auth/passkey/register/begin" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let row = ctx.store.get_by_id(&ctx.store.manifest().auth.user.entity, &user_id).ok().flatten();
        let user_name = row.as_ref().and_then(|r| r.get("email")).and_then(|v| v.as_str())
            .unwrap_or(&user_id).to_string();
        let challenge = ctx.passkeys.mint_challenge(user_id.clone(),
            pylon_auth::webauthn::ChallengeKind::Registration);
        let rp_id = std::env::var("PYLON_WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".into());
        return Some((200, serde_json::json!({
            "challenge": challenge, "rpId": rp_id, "userId": user_id, "userName": user_name
        }).to_string()));
    }
    if url == "/api/auth/passkey/register/finish" && method == HttpMethod::Post {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return Some((400, json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")))),
        };
        let challenge = data.get("challenge").and_then(|v| v.as_str()).unwrap_or("");
        let cred_id = data.get("credentialId").and_then(|v| v.as_str()).unwrap_or("");
        let public_key_b64 = data.get("publicKey").and_then(|v| v.as_str()).unwrap_or("");
        let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("passkey").to_string();
        if cred_id.is_empty() || public_key_b64.is_empty() {
            return Some((400, json_error("MISSING_FIELD", "credentialId + publicKey required")));
        }
        if ctx.passkeys.take_challenge(challenge,
            pylon_auth::webauthn::ChallengeKind::Registration).is_none() {
            return Some((401, json_error("BAD_CHALLENGE", "Challenge missing or expired")));
        }
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let public_key = match URL_SAFE_NO_PAD.decode(public_key_b64) {
            Ok(b) => b,
            Err(e) => return Some((400, json_error("BAD_PUBKEY", &format!("not base64url: {e}")))),
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        ctx.passkeys.store_passkey(pylon_auth::webauthn::Passkey {
            id: cred_id.to_string(), user_id, public_key, sign_count: 0,
            name, created_at: now, last_used_at: None,
        });
        return Some((200, serde_json::json!({"registered": true, "id": cred_id}).to_string()));
    }
    if url == "/api/auth/passkey/login/begin" && method == HttpMethod::Post {
        let challenge = ctx.passkeys.mint_challenge(String::new(),
            pylon_auth::webauthn::ChallengeKind::Assertion);
        let rp_id = std::env::var("PYLON_WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".into());
        return Some((200, serde_json::json!({"challenge": challenge, "rpId": rp_id}).to_string()));
    }
    if url == "/api/auth/passkey/login/finish" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return Some((400, json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")))),
        };
        let cred_id = data.get("credentialId").and_then(|v| v.as_str()).unwrap_or("");
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let auth_data = URL_SAFE_NO_PAD.decode(
            data.get("authenticatorData").and_then(|v| v.as_str()).unwrap_or("")).unwrap_or_default();
        let client_data = URL_SAFE_NO_PAD.decode(
            data.get("clientDataJSON").and_then(|v| v.as_str()).unwrap_or("")).unwrap_or_default();
        let sig = URL_SAFE_NO_PAD.decode(
            data.get("signature").and_then(|v| v.as_str()).unwrap_or("")).unwrap_or_default();
        let expected_origin = std::env::var("PYLON_WEBAUTHN_ORIGIN")
            .unwrap_or_else(|_| "https://localhost".into());
        let expected_rp_id = std::env::var("PYLON_WEBAUTHN_RP_ID")
            .unwrap_or_else(|_| "localhost".into());
        let input = pylon_auth::webauthn::AssertionInput {
            credential_id: cred_id, authenticator_data: &auth_data,
            client_data_json: &client_data, signature: &sig, user_handle: None,
        };
        let key = match pylon_auth::webauthn::verify_assertion(
            ctx.passkeys, &input, &expected_origin, &expected_rp_id, None) {
            Ok(k) => k,
            Err(e) => return Some((401, json_error("PASSKEY_VERIFY_FAILED", &e.to_string()))),
        };
        let session = ctx.session_store.create(key.user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((200, serde_json::json!({
            "token": session.token, "user_id": key.user_id, "expires_at": session.expires_at
        }).to_string()));
    }
    if url == "/api/auth/passkey/keys" && method == HttpMethod::Get {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u.to_string(),
            None => return Some((401, json_error("AUTH_REQUIRED", "Login required"))),
        };
        let payload: Vec<serde_json::Value> = ctx.passkeys.list_for_user(&user_id).iter()
            .map(|k| serde_json::json!({
                "id": k.id, "name": k.name, "created_at": k.created_at,
                "last_used_at": k.last_used_at,
            })).collect();
        return Some((200, serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into())));
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
        let auth = ctx.request_headers.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.as_str());
        if !pylon_auth::scim::check_bearer(auth) {
            return Some((401, serde_json::to_string(
                &pylon_auth::scim::ScimError::new(401, "missing or invalid SCIM bearer token")
            ).unwrap_or_default()));
        }
        let entity = &ctx.store.manifest().auth.user.entity;
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        match (parts.as_slice(), method) {
            // POST /scim/v2/Users
            (["Users"], HttpMethod::Post) => {
                let scim_user: pylon_auth::scim::ScimUser = match serde_json::from_str(body) {
                    Ok(u) => u,
                    Err(e) => return Some((400, serde_json::to_string(
                        &pylon_auth::scim::ScimError::new(400, &format!("invalid SCIM JSON: {e}"))
                    ).unwrap_or_default())),
                };
                let now = format!("{}Z", std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
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
                    Err(e) => return Some((409, serde_json::to_string(
                        &pylon_auth::scim::ScimError::new(409, &e.message)
                    ).unwrap_or_default())),
                }
            }
            (["Users"], HttpMethod::Get) => {
                let list = ctx.store.list(entity).unwrap_or_default();
                let users: Vec<pylon_auth::scim::ScimUser> = list.iter().filter_map(|row| {
                    let email = row.get("email").and_then(|v| v.as_str())?.to_string();
                    let id = row.get("id").and_then(|v| v.as_str()).map(String::from);
                    let active = row.get("scimActive").and_then(|v| v.as_bool()).unwrap_or(true);
                    let display_name = row.get("displayName").and_then(|v| v.as_str()).map(String::from);
                    Some(pylon_auth::scim::ScimUser {
                        id, user_name: email.clone(), active,
                        name: None,
                        emails: vec![pylon_auth::scim::ScimEmail {
                            value: email, primary: Some(true), kind: Some("work".into()),
                        }],
                        display_name,
                        schemas: vec!["urn:ietf:params:scim:schemas:core:2.0:User".into()],
                    })
                }).collect();
                return Some((200, serde_json::to_string(
                    &pylon_auth::scim::ScimListResponse::new(users)
                ).unwrap_or_default()));
            }
            (["Users", id], HttpMethod::Get) => {
                let row = match ctx.store.get_by_id(entity, id) {
                    Ok(Some(r)) => r,
                    _ => return Some((404, serde_json::to_string(
                        &pylon_auth::scim::ScimError::new(404, "user not found")
                    ).unwrap_or_default())),
                };
                let email = row.get("email").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let user = pylon_auth::scim::ScimUser {
                    id: Some(id.to_string()),
                    user_name: email.clone(),
                    active: row.get("scimActive").and_then(|v| v.as_bool()).unwrap_or(true),
                    name: None,
                    emails: vec![pylon_auth::scim::ScimEmail {
                        value: email, primary: Some(true), kind: Some("work".into()),
                    }],
                    display_name: row.get("displayName").and_then(|v| v.as_str()).map(String::from),
                    schemas: vec!["urn:ietf:params:scim:schemas:core:2.0:User".into()],
                };
                return Some((200, serde_json::to_string(&user).unwrap_or_default()));
            }
            (["Users", id], HttpMethod::Delete) => {
                // SCIM DELETE = soft delete (set scimActive=false). Hard
                // delete left to the host app's account-deletion flow.
                let _ = ctx.store.update(entity, id, &serde_json::json!({"scimActive": false}));
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
            ctx.request_headers.iter()
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
                kty: "RSA".into(), alg: "RS256".into(), use_: "sig".into(),
                kid, n, e,
            }]
        };
        return Some((200, serde_json::to_string(
            &pylon_auth::oidc_provider::Jwks { keys }
        ).unwrap_or_default()));
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
        let mode = match data.get("mode").and_then(|v| v.as_str()).unwrap_or("subscription") {
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
            Ok(s) => return Some((
                200,
                serde_json::json!({"url": s.url, "id": s.id}).to_string(),
            )),
            Err(e) => {
                tracing::warn!("[stripe] checkout create failed: {e}");
                return Some((502, json_error("STRIPE_FAILED", "Could not create checkout session")));
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
            None => return Some((501, json_error("WEBHOOK_NOT_CONFIGURED", "Set PYLON_STRIPE_WEBHOOK_SECRET"))),
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
        let event = match pylon_auth::stripe::verify_webhook(&secret, body.as_bytes(), sig_header, now) {
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
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } =
            rl.check(pylon_auth::rate_limit::AuthBucket::Send, ctx.peer_ip, Some(&email))
        {
            return Some((
                429,
                json_error_with_hint("RATE_LIMITED", "Too many reset requests",
                    &format!("Try again in {retry_after_secs}s")),
            ));
        }
        let entity = &ctx.store.manifest().auth.user.entity;
        if let Ok(Some(_)) = ctx.store.lookup(entity, "email", &email) {
            let minted = ctx.verification.mint(
                pylon_auth::verification::TokenKind::PasswordReset,
                &email,
                None,
                None,
            );
            let public_url = std::env::var("PYLON_PUBLIC_URL").unwrap_or_default();
            let reset_url = format!("{public_url}/reset-password?token={}", minted.plaintext);
            let body_text = format!(
                "Reset your password by visiting:\n\n{reset_url}\n\n\
                 This link expires in 30 minutes. If you didn't request a reset, ignore this email."
            );
            if let Err(e) = ctx.email.send(&email, "Reset your password", &body_text) {
                tracing::warn!("[auth] reset email to {} failed: {e}", redact_email(&email));
            }
        }
        // Always 200 — frontend says "if an account exists, we sent a link."
        return Some((200, serde_json::json!({"sent": true}).to_string()));
    }
    if url == "/api/auth/password/reset/complete" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return Some((400, json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")))),
        };
        let token = data.get("token").and_then(|v| v.as_str()).unwrap_or("");
        let new_password = data
            .get("newPassword")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if token.is_empty() || new_password.is_empty() {
            return Some((400, json_error("MISSING_FIELD", "token + newPassword required")));
        }
        if let Err(e) = pylon_auth::password::validate_length(new_password) {
            return Some((400, json_error("WEAK_PASSWORD", &e.to_string())));
        }
        if std::env::var("PYLON_DISABLE_HIBP").ok().as_deref() != Some("1") {
            if let Ok(n) = pylon_auth::password::check_pwned(new_password) {
                if n > 0 {
                    return Some((
                        400,
                        json_error_safe("PWNED_PASSWORD",
                            "This password has appeared in known data breaches.",
                            &format!("HIBP returned {n} occurrences")),
                    ));
                }
            }
        }
        let consumed = match ctx.verification.consume(
            token,
            pylon_auth::verification::TokenKind::PasswordReset,
        ) {
            Ok(t) => t,
            Err(e) => return Some((401, json_error("INVALID_TOKEN", &e.to_string()))),
        };
        let entity = &ctx.store.manifest().auth.user.entity;
        let row = match ctx.store.lookup(entity, "email", &consumed.email) {
            Ok(Some(r)) => r,
            _ => return Some((404, json_error("USER_NOT_FOUND", "Account no longer exists"))),
        };
        let user_id = row.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let new_hash = pylon_auth::password::hash_password(new_password);
        if let Err(e) = ctx.store.update(entity, &user_id, &serde_json::json!({"passwordHash": new_hash})) {
            return Some((400, json_error(&e.code, &e.message)));
        }
        // Revoke ALL existing sessions — same posture as password change.
        let revoked = ctx.session_store.revoke_all_for_user(&user_id);
        let session = ctx.session_store.create(user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((200, serde_json::json!({
            "reset": true, "revoked_sessions": revoked,
            "token": session.token, "user_id": user_id, "expires_at": session.expires_at,
        }).to_string()));
    }
    // ─── Magic links ──────────────────────────────────────────────────
    //
    // Like magic codes but the user clicks a URL instead of typing
    // a 6-digit code. /send mints + emails; /verify takes the token
    // (via either the GET `?token=` browser flow or POST JSON body)
    // and mints a session.
    if url == "/api/auth/magic-link/send" && method == HttpMethod::Post {
        let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let email = data.get("email").and_then(|v| v.as_str()).unwrap_or("").trim().to_lowercase();
        if email.is_empty() || !email.contains('@') {
            return Some((400, json_error("INVALID_EMAIL", "valid email required")));
        }
        let rl = pylon_auth::rate_limit::AuthRateLimiter::shared();
        if let pylon_auth::rate_limit::RateLimitDecision::Deny { retry_after_secs } =
            rl.check(pylon_auth::rate_limit::AuthBucket::Send, ctx.peer_ip, Some(&email))
        {
            return Some((
                429,
                json_error_with_hint("RATE_LIMITED", "Too many sign-in requests",
                    &format!("Try again in {retry_after_secs}s")),
            ));
        }
        // Optional CAPTCHA gate.
        if let Some(cfg) = pylon_auth::captcha::CaptchaConfig::from_env() {
            let token = data.get("captchaToken").and_then(|v| v.as_str()).unwrap_or("");
            if cfg.verify(token, Some(ctx.peer_ip)).is_err() {
                return Some((400, json_error("CAPTCHA_FAILED", "CAPTCHA failed")));
            }
        }
        let minted = ctx.verification.mint(
            pylon_auth::verification::TokenKind::MagicLink, &email, None, None);
        let public_url = std::env::var("PYLON_PUBLIC_URL").unwrap_or_default();
        let verify_url = format!("{public_url}/api/auth/magic-link/verify?token={}", minted.plaintext);
        let body_text = format!(
            "Click here to sign in:\n\n{verify_url}\n\n\
             This link expires in 15 minutes. If you didn't request it, ignore this email."
        );
        if let Err(e) = ctx.email.send(&email, "Sign in to your account", &body_text) {
            tracing::warn!("[auth] magic-link email to {} failed: {e}", redact_email(&email));
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
                data.get("token").and_then(|v| v.as_str()).unwrap_or("").to_string()
            };
            if token.is_empty() {
                return Some((400, json_error("MISSING_TOKEN", "token required")));
            }
            let consumed = match ctx.verification.consume(
                &token, pylon_auth::verification::TokenKind::MagicLink,
            ) {
                Ok(t) => t,
                Err(e) => return Some((401, json_error("INVALID_TOKEN", &e.to_string()))),
            };
            let entity = &ctx.store.manifest().auth.user.entity;
            let user_id = match ctx.store.lookup(entity, "email", &consumed.email) {
                Ok(Some(row)) => row.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                _ => {
                    let now = format!("{}Z", std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
                    match ctx.store.insert(entity, &serde_json::json!({
                        "email": consumed.email,
                        "displayName": consumed.email,
                        "emailVerified": now.clone(),
                        "createdAt": now,
                    })) {
                        Ok(id) => id,
                        Err(e) => return Some((400, json_error(&e.code, &e.message))),
                    }
                }
            };
            let session = ctx.session_store.create(user_id.clone());
            ctx.maybe_set_session_cookie(&session.token);
            // Browser flow → 302 to dashboard; SDK flow → JSON.
            if method == HttpMethod::Get {
                let dashboard = std::env::var("PYLON_DASHBOARD_URL").unwrap_or_else(|_| "/".into());
                ctx.add_response_header("Location", dashboard);
                return Some((302, String::new()));
            }
            return Some((200, serde_json::json!({
                "token": session.token, "user_id": user_id, "expires_at": session.expires_at
            }).to_string()));
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
            return Some((403, json_error("API_KEY_AUTH_FORBIDDEN", "Email change requires a session")));
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return Some((400, json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")))),
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
        if let Ok(Some(_)) = ctx.store.lookup(entity, "email", &new_email) {
            // Email is taken. Always 200 to avoid leaking which
            // emails are registered, but no email is actually sent.
            return Some((200, serde_json::json!({"sent": true}).to_string()));
        }
        let minted = ctx.verification.mint(
            pylon_auth::verification::TokenKind::EmailChange,
            &new_email,
            Some(user_id.clone()),
            Some(new_email.clone()),
        );
        let public_url = std::env::var("PYLON_PUBLIC_URL").unwrap_or_default();
        let confirm_url = format!("{public_url}/email-change/confirm?token={}", minted.plaintext);
        let body_text = format!(
            "Confirm your new email by visiting:\n\n{confirm_url}\n\n\
             This link expires in 24 hours. If you didn't request this change, ignore the email."
        );
        if let Err(e) = ctx.email.send(&new_email, "Confirm your email change", &body_text) {
            tracing::warn!("[auth] email-change confirm to {} failed: {e}", redact_email(&new_email));
        }
        return Some((200, serde_json::json!({"sent": true}).to_string()));
    }
    if url == "/api/auth/email/change/confirm" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return Some((400, json_error_safe("INVALID_JSON", "Invalid body", &format!("{e}")))),
        };
        let token = data.get("token").and_then(|v| v.as_str()).unwrap_or("");
        if token.is_empty() {
            return Some((400, json_error("MISSING_TOKEN", "token required")));
        }
        let consumed = match ctx.verification.consume(
            token, pylon_auth::verification::TokenKind::EmailChange,
        ) {
            Ok(t) => t,
            Err(e) => return Some((401, json_error("INVALID_TOKEN", &e.to_string()))),
        };
        let user_id = consumed.user_id.unwrap_or_default();
        let new_email = consumed.payload.unwrap_or_default();
        if user_id.is_empty() || new_email.is_empty() {
            return Some((400, json_error("INVALID_TOKEN", "token has no embedded user/email")));
        }
        let entity = &ctx.store.manifest().auth.user.entity;
        let now = format!("{}Z", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
        if let Err(e) = ctx.store.update(entity, &user_id, &serde_json::json!({
            "email": new_email, "emailVerified": now,
        })) {
            return Some((400, json_error(&e.code, &e.message)));
        }
        // Revoke other sessions on email change — same blast-radius
        // posture as password change.
        ctx.session_store.revoke_all_for_user(&user_id);
        let session = ctx.session_store.create(user_id.clone());
        ctx.maybe_set_session_cookie(&session.token);
        return Some((200, serde_json::json!({
            "changed": true, "email": new_email,
            "token": session.token, "user_id": user_id, "expires_at": session.expires_at,
        }).to_string()));
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
            return Some((403, json_error("API_KEY_AUTH_FORBIDDEN", "Backup codes require a session")));
        }
        // 10 random codes, formatted XXXX-XXXX (8 alphanumeric chars
        // with a dash for readability — Google's standard).
        use rand::Rng;
        let mut rng = rand::thread_rng();
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let codes: Vec<String> = (0..10).map(|_| {
            let raw: String = (0..8).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect();
            format!("{}-{}", &raw[..4], &raw[4..])
        }).collect();
        // Store the SHA-256 hashes (HMAC w/ pepper would be even
        // better, but matching the existing api-key pattern).
        use sha2::{Digest, Sha256};
        let hashes: Vec<String> = codes.iter().map(|c| {
            let mut h = Sha256::new();
            h.update(c.as_bytes());
            let out = h.finalize();
            use std::fmt::Write;
            let mut s = String::with_capacity(64);
            for b in out { let _ = write!(s, "{b:02x}"); }
            s
        }).collect();
        return match ctx.store.update(
            &ctx.store.manifest().auth.user.entity, &user_id,
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
        let session = ctx.session_store.create_guest();
        ctx.maybe_set_session_cookie(&session.token);
        return Some((200, serde_json::json!({
            "token": session.token, "user_id": session.user_id,
            "is_guest": true, "expires_at": session.expires_at,
        }).to_string()));
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
        // Delete the user row.
        match ctx
            .store
            .delete(&ctx.store.manifest().auth.user.entity, &user_id)
        {
            Ok(_) => {}
            Err(e) => return Some((400, json_error(&e.code, &e.message))),
        }
        ctx.add_response_header("Set-Cookie", ctx.cookie_config.clear_value());
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
