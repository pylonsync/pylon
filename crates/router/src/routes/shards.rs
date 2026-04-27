//! Real-time simulation shards (games, MMO zones, live docs, etc.).
//! `/api/shards` (admin list), `POST /api/shards/<id>/input`,
//! `GET /api/shards/<id>` (per-shard info).

use crate::{json_error, parse_json, query_param, require_admin, RouterContext};
use pylon_http::HttpMethod;

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
                let ids = s.list_shards();
                let out: Vec<serde_json::Value> = ids
                    .iter()
                    .map(|id| {
                        s.get_shard(id)
                            .map(|sh| {
                                serde_json::json!({
                                    "id": sh.id(),
                                    "running": sh.is_running(),
                                    "tick": sh.tick_number(),
                                    "subscribers": sh.subscriber_count(),
                                    "input_queue": sh.input_queue_len(),
                                })
                            })
                            .unwrap_or(serde_json::json!({"id": id}))
                    })
                    .collect();
                (
                    200,
                    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into()),
                )
            }
            None => (200, "[]".into()),
        });
    }

    // POST /api/shards/:id/input
    if method == HttpMethod::Post {
        if let Some(rest) = url.strip_prefix("/api/shards/") {
            let rest = rest.split('?').next().unwrap_or(rest);
            if let Some(shard_id) = rest.strip_suffix("/input") {
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

                let shard_auth = pylon_realtime::ShardAuth {
                    user_id: ctx.auth_ctx.user_id.clone(),
                    is_admin: ctx.auth_ctx.is_admin,
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
                        Err(e) => (400, json_error("INPUT_REJECTED", &e.to_string())),
                    },
                );
            }
        }
    }

    // GET /api/shards/:id (per-shard info)
    if method == HttpMethod::Get {
        if let Some(shard_id) = url.strip_prefix("/api/shards/") {
            let shard_id = shard_id.split('?').next().unwrap_or(shard_id);
            if !shard_id.is_empty() && !shard_id.contains('/') {
                if let Some(shards) = ctx.shards {
                    if let Some(sh) = shards.get_shard(shard_id) {
                        return Some((
                            200,
                            serde_json::json!({
                                "id": sh.id(),
                                "running": sh.is_running(),
                                "tick": sh.tick_number(),
                                "subscribers": sh.subscriber_count(),
                                "input_queue": sh.input_queue_len(),
                            })
                            .to_string(),
                        ));
                    }
                    return Some((
                        404,
                        json_error(
                            "SHARD_NOT_FOUND",
                            &format!("Shard \"{shard_id}\" not found"),
                        ),
                    ));
                }
            }
        }
    }

    None
}
