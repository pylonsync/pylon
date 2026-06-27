//! TypeScript function endpoints: list (admin), traces (admin),
//! webhook actions (`/api/webhooks/<name>`), function invoke
//! (`/api/fn/<name>`).
//!
//! Webhook + function rate limits bucket unauth callers by peer_ip,
//! not by the literal "anon" string — without this every unauth caller
//! worldwide shared one bucket per (action, "anon") and a single
//! attacker could starve all anon traffic.

use crate::{json_error, parse_json, query_param, require_admin, RouterContext};
use pylon_auth::AuthContext;
use pylon_functions::registry::FnAuthMode;
use pylon_http::HttpMethod;

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[test]
    fn public_allows_anyone() {
        // Anonymous, guest, user, admin — all four pass `public`.
        assert_eq!(
            check_fn_auth(FnAuthMode::Public, &AuthContext::anonymous()),
            FnAuthGate::Allowed
        );
        assert_eq!(
            check_fn_auth(FnAuthMode::Public, &AuthContext::guest("g1".into())),
            FnAuthGate::Allowed
        );
        assert_eq!(
            check_fn_auth(FnAuthMode::Public, &AuthContext::authenticated("u1".into())),
            FnAuthGate::Allowed
        );
        assert_eq!(
            check_fn_auth(FnAuthMode::Public, &AuthContext::admin()),
            FnAuthGate::Allowed
        );
    }

    #[test]
    fn guest_rejects_anonymous_but_allows_session_holders() {
        // Anonymous has no user_id at all — bounce with NeedsGuest.
        assert_eq!(
            check_fn_auth(FnAuthMode::Guest, &AuthContext::anonymous()),
            FnAuthGate::NeedsGuest
        );
        // Guest session — user_id present, is_guest=true → allowed.
        assert_eq!(
            check_fn_auth(FnAuthMode::Guest, &AuthContext::guest("g1".into())),
            FnAuthGate::Allowed
        );
        // Real user — also allowed (superset).
        assert_eq!(
            check_fn_auth(FnAuthMode::Guest, &AuthContext::authenticated("u1".into())),
            FnAuthGate::Allowed
        );
    }

    #[test]
    fn user_rejects_anonymous_and_guest_but_allows_real_users() {
        assert_eq!(
            check_fn_auth(FnAuthMode::User, &AuthContext::anonymous()),
            FnAuthGate::NeedsAuth
        );
        // Crucial: a guest session is NOT a real user. This is the
        // exact bug `auth: "user"` is meant to prevent — handlers
        // that assumed user_id meant "real user" but accepted guest
        // ids. Default-deny + guest-rejects closes the gap.
        assert_eq!(
            check_fn_auth(FnAuthMode::User, &AuthContext::guest("g1".into())),
            FnAuthGate::NeedsAuth
        );
        assert_eq!(
            check_fn_auth(FnAuthMode::User, &AuthContext::authenticated("u1".into())),
            FnAuthGate::Allowed
        );
    }

    #[test]
    fn admin_rejects_everyone_but_admins() {
        assert_eq!(
            check_fn_auth(FnAuthMode::Admin, &AuthContext::anonymous()),
            FnAuthGate::NeedsAdmin
        );
        assert_eq!(
            check_fn_auth(FnAuthMode::Admin, &AuthContext::guest("g1".into())),
            FnAuthGate::NeedsAdmin
        );
        assert_eq!(
            check_fn_auth(FnAuthMode::Admin, &AuthContext::authenticated("u1".into())),
            FnAuthGate::NeedsAdmin
        );
        assert_eq!(
            check_fn_auth(FnAuthMode::Admin, &AuthContext::admin()),
            FnAuthGate::Allowed
        );
    }

    #[test]
    fn admin_bypasses_every_mode() {
        // Mirrors the policy-engine convention — operators must be
        // able to invoke any function from ops scripts without
        // wildcard checks on every def.
        for mode in [
            FnAuthMode::Public,
            FnAuthMode::Guest,
            FnAuthMode::User,
            FnAuthMode::Admin,
        ] {
            assert_eq!(
                check_fn_auth(mode, &AuthContext::admin()),
                FnAuthGate::Allowed,
                "admin should bypass mode {:?}",
                mode,
            );
        }
    }
}

/// Outcome of the function-level auth gate. Mirrors the four
/// possible bodies the route handler will emit if the gate fires.
///
/// Extracted into a pure function so the gate is testable without
/// the full RouterContext / TCP harness — the route handler
/// translates `GateOutcome` into `(status, body)` after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FnAuthGate {
    /// Caller satisfies the declared auth mode; proceed to invoke.
    Allowed,
    /// Caller is anonymous; function requires a real signed-in user.
    /// Status 401, code `AUTH_REQUIRED`.
    NeedsAuth,
    /// Caller has no session at all; function allows guests but
    /// the request didn't even carry a guest cookie. Status 401,
    /// code `AUTH_REQUIRED`, hint mentions /api/auth/guest.
    NeedsGuest,
    /// Caller is authenticated but not admin; function is admin-gated.
    /// Status 403, code `FORBIDDEN`.
    NeedsAdmin,
}

/// Pure auth-mode gate. Admin context bypasses every mode
/// (matches the policy-engine convention — admin bypasses
/// policies too, so ops scripts work everywhere without
/// wildcard rules).
pub(crate) fn check_fn_auth(mode: FnAuthMode, auth_ctx: &AuthContext) -> FnAuthGate {
    if auth_ctx.is_admin {
        return FnAuthGate::Allowed;
    }
    match mode {
        FnAuthMode::Public => FnAuthGate::Allowed,
        FnAuthMode::Guest => {
            // Guest sessions OR real users both pass. The discriminator
            // is "did the request carry a session at all" — represented
            // by `user_id.is_some()` (a guest session has user_id =
            // guest_id, not None).
            if auth_ctx.user_id.is_some() {
                FnAuthGate::Allowed
            } else {
                FnAuthGate::NeedsGuest
            }
        }
        FnAuthMode::User => {
            if auth_ctx.is_authenticated() {
                FnAuthGate::Allowed
            } else {
                FnAuthGate::NeedsAuth
            }
        }
        FnAuthMode::Admin => FnAuthGate::NeedsAdmin,
    }
}

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    // GET /api/fn — admin-only enumeration of registered functions.
    if url == "/api/fn" && method == HttpMethod::Get {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        return Some(match ctx.functions {
            Some(f) => (
                200,
                serde_json::to_string(&f.list_fns()).unwrap_or_else(|_| "[]".into()),
            ),
            None => (200, "[]".into()),
        });
    }

    // GET /api/fn/traces — admin-only execution traces.
    if url.starts_with("/api/fn/traces") && method == HttpMethod::Get {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        return Some(match ctx.functions {
            Some(f) => {
                let limit: usize = query_param(url, "limit")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(50)
                    .min(500);
                let traces = f.recent_traces(limit);
                (
                    200,
                    serde_json::to_string(&traces).unwrap_or_else(|_| "[]".into()),
                )
            }
            None => (200, "[]".into()),
        });
    }

    // /api/webhooks/:action_name — invoke an action with full request
    // context (raw method/path/headers/body). Use for signed-payload
    // webhooks (Stripe, GitHub, Slack).
    if let Some(action_name) = url.strip_prefix("/api/webhooks/") {
        let action_name = action_name.split('?').next().unwrap_or(action_name);
        if !action_name.is_empty() {
            let fn_ops = match ctx.functions {
                Some(f) => f,
                None => {
                    return Some((
                        503,
                        json_error(
                            "FUNCTIONS_NOT_AVAILABLE",
                            "TypeScript function runtime is not configured",
                        ),
                    ));
                }
            };
            let def = match fn_ops.get_fn(action_name) {
                Some(d) if !d.internal => d,
                _ => {
                    // Internal-only actions get the same 404 shape as
                    // "not registered" so probing can't enumerate them.
                    return Some((
                        404,
                        json_error(
                            "FN_NOT_FOUND",
                            &format!("Action \"{action_name}\" is not registered"),
                        ),
                    ));
                }
            };
            // Only actions can be webhook targets — mutations run under
            // a write tx, queries are read-only. Action = "external I/O,
            // non-transactional".
            if def.fn_type != pylon_functions::protocol::FnType::Action {
                return Some((
                    400,
                    json_error(
                        "NOT_AN_ACTION",
                        &format!(
                            "\"{action_name}\" is not an action — only actions can be webhook targets"
                        ),
                    ),
                ));
            }

            let auth = pylon_functions::protocol::AuthInfo {
                user_id: ctx.auth_ctx.user_id.clone(),
                is_admin: ctx.auth_ctx.is_admin,
                tenant_id: ctx.auth_ctx.tenant_id.clone(),
                roles: ctx.auth_ctx.roles.clone(),
            };

            let identity = auth.user_id.as_deref().unwrap_or_else(|| {
                if ctx.peer_ip.is_empty() {
                    "anon"
                } else {
                    ctx.peer_ip
                }
            });
            if let Err(retry_after) = fn_ops.check_rate_limit(action_name, identity) {
                let body = format!(
                    r#"{{"error":{{"code":"RATE_LIMITED","message":"Webhook \"{action_name}\" rate limit exceeded","retry_after_secs":{retry_after}}}}}"#
                );
                return Some((429, body));
            }

            let mut headers = std::collections::HashMap::new();
            for (name, value) in ctx.request_headers {
                headers
                    .entry(name.to_ascii_lowercase())
                    .and_modify(|existing: &mut String| {
                        existing.push_str(", ");
                        existing.push_str(value);
                    })
                    .or_insert_with(|| value.clone());
            }
            let request = pylon_functions::protocol::RequestInfo {
                method: format!("{:?}", method).to_uppercase(),
                path: url.to_string(),
                headers,
                raw_body: body.to_string(),
            };

            let args = serde_json::json!({ "rawBody": body });

            return Some(
                match fn_ops.call(action_name, args, auth, None, Some(request)) {
                    Ok((value, _trace)) => (
                        200,
                        serde_json::to_string(&value).unwrap_or_else(|_| "null".into()),
                    ),
                    Err(e) => (400, json_error(&e.code, &e.message)),
                },
            );
        }
    }

    // POST /api/fn/:name
    if let Some(fn_name) = url.strip_prefix("/api/fn/") {
        let fn_name = fn_name.split('?').next().unwrap_or(fn_name);
        if method == HttpMethod::Post && !fn_name.is_empty() && fn_name != "traces" {
            let fn_ops = match ctx.functions {
                Some(f) => f,
                None => {
                    return Some((
                        503,
                        json_error(
                            "FUNCTIONS_NOT_AVAILABLE",
                            "TypeScript function runtime is not configured",
                        ),
                    ));
                }
            };

            // Refuse internal-only functions over HTTP for non-admin
            // callers. The TS define API exposes `internal: true` for
            // helper mutations that shouldn't be reachable directly
            // (e.g. ones that trust their args without re-checking
            // caller authority because they assume a wrapping action
            // verified everything). Same 404 shape as "not registered"
            // — probing can't confirm the name exists.
            //
            // Admin sessions (PYLON_ADMIN_TOKEN, or `auth.user.adminField`
            // sessions, or the Studio admin cookie) bypass the gate so
            // operators can manually kick crons + debug helper functions
            // from Studio without round-tripping through the scheduler.
            let fn_def = match fn_ops.get_fn(fn_name) {
                Some(def) if def.internal && !ctx.auth_ctx.is_admin => {
                    return Some((
                        404,
                        json_error(
                            "FN_NOT_FOUND",
                            &format!("Function \"{fn_name}\" is not registered"),
                        ),
                    ));
                }
                None => {
                    return Some((
                        404,
                        json_error(
                            "FN_NOT_FOUND",
                            &format!("Function \"{fn_name}\" is not registered"),
                        ),
                    ));
                }
                Some(def) => def,
            };

            // Declarative auth gate, enforced BEFORE the TS handler
            // runs. Forgetting an `if (!ctx.auth.userId) throw …`
            // check no longer leaks data — when the def declared
            // `auth: "user"` (the default), an anonymous request
            // never reaches the handler in the first place.
            //
            // Admin sessions bypass every mode (see `check_fn_auth`)
            // so ops scripts and Studio can invoke functions without
            // wildcard "admin can call this" checks on every def.
            match check_fn_auth(fn_def.auth, ctx.auth_ctx) {
                FnAuthGate::Allowed => {}
                FnAuthGate::NeedsAuth => {
                    return Some((
                        401,
                        json_error(
                            "AUTH_REQUIRED",
                            &format!("Function \"{fn_name}\" requires a signed-in user"),
                        ),
                    ));
                }
                FnAuthGate::NeedsGuest => {
                    return Some((
                        401,
                        json_error(
                            "AUTH_REQUIRED",
                            &format!(
                                "Function \"{fn_name}\" requires a session — sign in or POST /api/auth/guest first"
                            ),
                        ),
                    ));
                }
                FnAuthGate::NeedsAdmin => {
                    return Some((
                        403,
                        json_error(
                            "FORBIDDEN",
                            &format!("Function \"{fn_name}\" requires admin authentication"),
                        ),
                    ));
                }
            }

            let args: serde_json::Value = if body.trim().is_empty() {
                serde_json::json!({})
            } else {
                match parse_json(body) {
                    Ok(v) => v,
                    Err((s, b)) => return Some((s, b)),
                }
            };

            let auth = pylon_functions::protocol::AuthInfo {
                user_id: ctx.auth_ctx.user_id.clone(),
                is_admin: ctx.auth_ctx.is_admin,
                tenant_id: ctx.auth_ctx.tenant_id.clone(),
                roles: ctx.auth_ctx.roles.clone(),
            };

            let identity = auth.user_id.as_deref().unwrap_or_else(|| {
                if ctx.peer_ip.is_empty() {
                    "anon"
                } else {
                    ctx.peer_ip
                }
            });
            if let Err(retry_after) = fn_ops.check_rate_limit(fn_name, identity) {
                let body = format!(
                    r#"{{"error":{{"code":"RATE_LIMITED","message":"Function \"{fn_name}\" rate limit exceeded","retry_after_secs":{retry_after}}}}}"#
                );
                return Some((429, body));
            }

            // Synthesize RequestInfo so action handlers invoked via
            // /api/fn/<name> can inspect raw body + headers. Without
            // this, webhook receivers (GitHub / Stripe / Slack) that
            // need HMAC verification of the EXACT signed bytes are
            // dead — `ctx.request` reads as undefined and the handler
            // can't reach the raw body. Queries + mutations don't have
            // ctx.request in their type signature, so passing this is
            // a no-op for them.
            let request_info = pylon_functions::protocol::RequestInfo {
                method: "POST".to_string(),
                path: url.to_string(),
                headers: ctx
                    .request_headers
                    .iter()
                    .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
                    .collect(),
                raw_body: body.to_string(),
            };

            // Bracket the action with the change-log's seq counter so
            // we can tell the SDK "this action generated events up to
            // seq N." The SDK uses that to short-circuit the latency
            // window between this HTTP response landing and the WS
            // broadcast of the same events: if useQuery hasn't seen
            // seq N yet, the SDK pulls immediately instead of waiting
            // for the next periodic poll. Kills the need for app
            // code to call `refetch()` after every mutation.
            let pre_seq = ctx.change_log.current_seq();
            let result = fn_ops.call(fn_name, args, auth, None, Some(request_info));
            let post_seq = ctx.change_log.current_seq();
            if post_seq > pre_seq {
                ctx.add_response_header("X-Pylon-Change-Seq", post_seq.to_string());
            }
            return Some(match result {
                Ok((value, _trace)) => (
                    200,
                    serde_json::to_string(&value).unwrap_or_else(|_| "null".into()),
                ),
                Err(e) => (400, json_error(&e.code, &e.message)),
            });
        }
    }

    None
}
