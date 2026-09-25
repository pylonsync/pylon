//! Real-time simulation shards (games, MMO zones, live docs, etc.).
//!
//! - `GET /api/shards`: every shard with its numbers (admin).
//! - `GET /api/shards/<id>`: one shard's numbers and subscribers (admin).
//! - `POST /api/shards/<id>/stop`: stop a shard (admin).
//! - `POST /api/shards/<id>/kick` `{ "subscriber": id }`: close a
//!   subscriber's connections (admin).
//! - `POST /api/shards/<id>/input`: send an input.

use crate::{
    json_error, parse_json, query_decode, query_param, require_admin, RouterContext, ShardOps,
};
use pylon_http::HttpMethod;

/// One shard as the admin routes report it.
fn describe(shards: &dyn ShardOps, sh: &dyn pylon_realtime::DynShard) -> serde_json::Value {
    let id = sh.id();
    serde_json::json!({
        "id": id,
        "kind": shards.shard_kind(id),
        "running": sh.is_running(),
        "error": shards.shard_failure(id),
        "tick": sh.tick_number(),
        "subscribers": sh.subscriber_count(),
        "input_queue": sh.input_queue_len(),
        "stats": sh.stats(),
    })
}

fn not_found(shard_id: &str) -> (u16, String) {
    (
        404,
        json_error(
            "SHARD_NOT_FOUND",
            &format!("Shard \"{shard_id}\" not found"),
        ),
    )
}

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    // GET /api/shards (admin-only enumeration).
    if url == "/api/shards" && method == HttpMethod::Get {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        return Some(match ctx.shards {
            Some(s) => {
                let mut ids = s.list_shards();
                ids.sort();
                let out: Vec<serde_json::Value> = ids
                    .iter()
                    .filter_map(|id| s.get_shard(id))
                    .map(|sh| describe(s, sh.as_ref()))
                    .collect();
                (
                    200,
                    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into()),
                )
            }
            None => (200, "[]".into()),
        });
    }

    // POST /api/shards/:id/stop and /kick (admin)
    if method == HttpMethod::Post {
        if let Some(rest) = url.strip_prefix("/api/shards/") {
            let rest = rest.split('?').next().unwrap_or(rest);
            let action = rest
                .strip_suffix("/stop")
                .map(|id| (id, "stop"))
                .or_else(|| rest.strip_suffix("/kick").map(|id| (id, "kick")));
            if let Some((shard_id, action)) = action {
                // Shard ids may hold `:`, which a client encodes.
                let shard_id = query_decode(shard_id);
                let shard_id = shard_id.as_str();
                if let Some(err) = require_admin(ctx) {
                    return Some(err);
                }
                let Some(shards) = ctx.shards else {
                    return Some(not_found(shard_id));
                };
                if action == "stop" {
                    return Some(if shards.stop_shard(shard_id) {
                        (200, serde_json::json!({ "stopped": true }).to_string())
                    } else {
                        not_found(shard_id)
                    });
                }
                let Some(shard) = shards.get_shard(shard_id) else {
                    return Some(not_found(shard_id));
                };
                let body: serde_json::Value = match parse_json(body) {
                    Ok(v) => v,
                    Err((s, b)) => return Some((s, b)),
                };
                let Some(subscriber) = body.get("subscriber").and_then(|v| v.as_str()) else {
                    return Some((
                        400,
                        json_error("MISSING_SUBSCRIBER", "body needs { \"subscriber\": id }"),
                    ));
                };
                let removed =
                    shard.remove_subscriber(&pylon_realtime::SubscriberId::new(subscriber));
                return Some(if removed {
                    (200, serde_json::json!({ "kicked": true }).to_string())
                } else {
                    (
                        404,
                        json_error(
                            "SUBSCRIBER_NOT_FOUND",
                            &format!("No subscriber \"{subscriber}\" in shard \"{shard_id}\""),
                        ),
                    )
                });
            }
        }
    }

    // POST /api/shards/:id/input
    if method == HttpMethod::Post {
        if let Some(rest) = url.strip_prefix("/api/shards/") {
            let rest = rest.split('?').next().unwrap_or(rest);
            if let Some(shard_id) = rest.strip_suffix("/input") {
                let shard_id = query_decode(shard_id);
                let shard_id = shard_id.as_str();
                let shards = match ctx.shards {
                    Some(s) => s,
                    None => {
                        return Some((
                            503,
                            json_error("SHARDS_NOT_AVAILABLE", "Shard system is not configured"),
                        ));
                    }
                };
                let shard = match shards.get_shard(shard_id) {
                    Some(s) => s,
                    None => {
                        return Some((
                            404,
                            json_error(
                                "SHARD_NOT_FOUND",
                                &format!("Shard \"{shard_id}\" not found"),
                            ),
                        ));
                    }
                };

                let envelope: serde_json::Value = match parse_json(body) {
                    Ok(v) => v,
                    Err((s, b)) => return Some((s, b)),
                };
                let subscriber_id = envelope
                    .get("subscriber_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| ctx.auth_ctx.user_id.clone())
                    .unwrap_or_else(|| format!("anon_{}", query_param(url, "sid").unwrap_or("0")));
                let client_seq = envelope.get("client_seq").and_then(|v| v.as_u64());
                let input = envelope
                    .get("input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let input_str = serde_json::to_string(&input).unwrap_or_else(|_| "null".into());

                // A shard ticket (X-Pylon-Shard-Ticket) carries the claims
                // authorize_input may need, as on the WebSocket.
                let ticket = match ctx
                    .request_headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("x-pylon-shard-ticket"))
                    .map(|(_, v)| shards.verify_ticket(v))
                {
                    Some(Ok(t)) => Some(t),
                    Some(Err(e)) => {
                        return Some((401, json_error("INVALID_TICKET", &e.to_string())))
                    }
                    None => None,
                };
                let shard_auth = pylon_realtime::ShardAuth {
                    user_id: ctx.auth_ctx.user_id.clone(),
                    is_admin: ctx.auth_ctx.is_admin,
                    roles: ctx.auth_ctx.roles.clone(),
                    tenant_id: ctx.auth_ctx.tenant_id.clone(),
                    ticket,
                };
                return Some(
                    match shard.push_input_json(
                        pylon_realtime::SubscriberId::new(subscriber_id),
                        &input_str,
                        client_seq,
                        &shard_auth,
                    ) {
                        Ok(seq) => (
                            200,
                            serde_json::json!({"accepted": true, "seq": seq}).to_string(),
                        ),
                        Err(pylon_realtime::ShardError::Unauthorized(reason)) => {
                            (403, json_error("UNAUTHORIZED", &reason))
                        }
                        // Over this subscriber's input limit, or the shard's
                        // whole queue is full: slow down and retry.
                        Err(
                            e @ (pylon_realtime::ShardError::InputRateLimited
                            | pylon_realtime::ShardError::InputQueueFull),
                        ) => (429, json_error("INPUT_RATE_LIMITED", &e.to_string())),
                        Err(e) => (400, json_error("INPUT_REJECTED", &e.to_string())),
                    },
                );
            }
        }
    }

    // GET /api/shards/:id (admin): numbers and subscribers
    if method == HttpMethod::Get {
        if let Some(shard_id) = url.strip_prefix("/api/shards/") {
            let shard_id = shard_id.split('?').next().unwrap_or(shard_id);
            if !shard_id.is_empty() && !shard_id.contains('/') {
                if let Some(err) = require_admin(ctx) {
                    return Some(err);
                }
                let shard_id = query_decode(shard_id);
                let shard_id = shard_id.as_str();
                let Some(shards) = ctx.shards else {
                    return Some(not_found(shard_id));
                };
                let Some(sh) = shards.get_shard(shard_id) else {
                    return Some(not_found(shard_id));
                };
                let mut out = describe(shards, sh.as_ref());
                out["subscriber_ids"] = sh
                    .subscriber_ids()
                    .into_iter()
                    .map(|(id, connections)| {
                        serde_json::json!({ "id": id.as_str(), "connections": connections })
                    })
                    .collect();
                return Some((200, out.to_string()));
            }
        }
    }

    None
}
