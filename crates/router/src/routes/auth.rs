//! `/api/auth/*` routes — sessions, OAuth, magic-link, password,
//! email verification, /me, /providers, /sessions, refresh.
//!
//! Returns `Some((status, body))` if this module owns the route, or
//! `None` to fall through to the next module. Behavior is identical
//! to the pre-split inline handlers in `lib.rs`; this is a pure
//! mechanical extraction so security audits can scope to one file.

use crate::{
    complete_oauth_login, json_error, json_error_safe, json_error_with_hint, parse_query,
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
    // Returns the resolved AuthContext the runtime computed for this
    // request, NOT a fresh `session_store.resolve(token)` lookup. The
    // distinction matters: PYLON_ADMIN_TOKEN bearer auth is resolved
    // by the runtime (server.rs) before route() is called, and the
    // result lives in `ctx.auth_ctx`. Calling session_store.resolve
    // here would skip the admin-token branch entirely and report
    // every admin caller as anonymous — which Studio relied on to
    // decide whether to surface admin tabs and which the dashboard
    // layout used to gate `/dashboard`. Trust the runtime's resolution.
    if url == "/api/auth/me" && method == HttpMethod::Get {
        return Some((
            200,
            serde_json::to_string(ctx.auth_ctx).unwrap_or_else(|_| "{}".into()),
        ));
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
                let user_id = match ctx.store.lookup("User", "email", email) {
                    Ok(Some(row)) => {
                        let id = row["id"].as_str().unwrap_or("").to_string();
                        // Magic-link login implicitly verifies the
                        // email — the caller proved control by typing
                        // the code we sent there. Stamp emailVerified
                        // if not already set.
                        if row.get("emailVerified").map_or(true, |v| v.is_null()) {
                            let _ = ctx.store.update(
                                "User",
                                &id,
                                &serde_json::json!({ "emailVerified": now }),
                            );
                        }
                        id
                    }
                    _ => ctx
                        .store
                        .insert(
                            "User",
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
        let user = match ctx.store.get_by_id("User", user_id) {
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
        let user = match ctx.store.get_by_id("User", user_id) {
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
                    "User",
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
        if password.len() < 8 {
            return Some((
                400,
                json_error("WEAK_PASSWORD", "password must be at least 8 characters"),
            ));
        }
        let display_name = data
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or(email.as_str())
            .to_string();

        if let Ok(Some(_)) = ctx.store.lookup("User", "email", &email) {
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
            "User",
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

        let row = ctx.store.lookup("User", "email", &email).ok().flatten();
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
        let providers: Vec<serde_json::Value> = ["google", "github"]
            .iter()
            .filter_map(|p| {
                registry.get(p).map(|c| {
                    serde_json::json!({
                        "provider": p,
                        "auth_url": c.auth_url(),
                    })
                })
            })
            .collect();
        return Some((
            200,
            serde_json::to_string(&providers).unwrap_or_else(|_| "[]".into()),
        ));
    }

    // GET /api/auth/login/:provider
    if let Some(provider_raw) = url.strip_prefix("/api/auth/login/") {
        let provider = provider_raw.split('?').next().unwrap_or(provider_raw);
        if method == HttpMethod::Get {
            let registry = pylon_auth::OAuthRegistry::from_env();
            if let Some(config) = registry.get(provider) {
                let state = ctx.oauth_state.create(provider);
                let redirect = config.auth_url_with_state(&state);
                let query = provider_raw.split_once('?').map(|(_, q)| q).unwrap_or("");
                let want_redirect = parse_query(query)
                    .get("redirect")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                if want_redirect {
                    ctx.add_response_header("Location", redirect);
                    return Some((302, String::new()));
                }
                return Some((
                    200,
                    serde_json::json!({"redirect": redirect, "state": state}).to_string(),
                ));
            }
            return Some((
                404,
                json_error_with_hint(
                    "PROVIDER_NOT_FOUND",
                    &format!("OAuth provider \"{provider}\" is not configured"),
                    "Set PYLON_OAUTH_GOOGLE_CLIENT_ID / PYLON_OAUTH_GITHUB_CLIENT_ID environment variables",
                ),
            ));
        }
    }

    // /api/auth/callback/:provider
    if let Some(provider_raw) = url.strip_prefix("/api/auth/callback/") {
        let provider = provider_raw.split('?').next().unwrap_or(provider_raw);
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
            let state = data.get("state").and_then(|v| v.as_str());
            let code = data.get("code").and_then(|v| v.as_str());
            let dev_email = data.get("email").and_then(|v| v.as_str());
            let dev_name = data.get("name").and_then(|v| v.as_str());
            return Some(
                match complete_oauth_login(ctx, provider, state, code, dev_email, dev_name) {
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
                },
            );
        }
        if method == HttpMethod::Get {
            let dashboard = match std::env::var("PYLON_DASHBOARD_URL") {
                Ok(v) if !v.is_empty() => v,
                _ => {
                    return Some((
                        500,
                        json_error(
                            "OAUTH_DASHBOARD_URL_MISSING",
                            "GET /api/auth/callback/:provider requires PYLON_DASHBOARD_URL to know where to redirect after sign-in",
                        ),
                    ));
                }
            };
            let query = provider_raw.split_once('?').map(|(_, q)| q).unwrap_or("");
            let params = parse_query(query);
            let state = params.get("state").map(String::as_str);
            let code = params.get("code").map(String::as_str);
            match complete_oauth_login(ctx, provider, state, code, None, None) {
                Ok((_user_id, session)) => {
                    let cookie_value = ctx.cookie_config.set_value(&session.token);
                    ctx.add_response_header("Set-Cookie", cookie_value);
                    ctx.add_response_header("Location", dashboard);
                    return Some((302, String::new()));
                }
                Err(err) => {
                    let target = format!(
                        "{}/login?oauth_error={}",
                        dashboard.trim_end_matches('/'),
                        url_encode(err.code)
                    );
                    ctx.add_response_header("Location", target);
                    return Some((302, String::new()));
                }
            }
        }
    }

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

    None
}
