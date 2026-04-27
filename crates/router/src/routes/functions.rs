//! TypeScript function endpoints: list (admin), traces (admin),
//! webhook actions (`/api/webhooks/<name>`), function invoke
//! (`/api/fn/<name>`).
//!
//! Webhook + function rate limits bucket unauth callers by peer_ip,
//! not by the literal "anon" string — without this every unauth caller
//! worldwide shared one bucket per (action, "anon") and a single
//! attacker could starve all anon traffic.

use crate::{json_error, parse_json, query_param, require_admin, RouterContext};
use pylon_http::HttpMethod;

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
                Some(d) => d,
                None => {
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

            if fn_ops.get_fn(fn_name).is_none() {
                return Some((
                    404,
                    json_error(
                        "FN_NOT_FOUND",
                        &format!("Function \"{fn_name}\" is not registered"),
                    ),
                ));
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

            return Some(match fn_ops.call(fn_name, args, auth, None, None) {
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
