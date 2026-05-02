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
        let registry = pylon_auth::OAuthRegistry::from_env();
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
            let registry = pylon_auth::OAuthRegistry::from_env();
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
