//! `/api/rooms/*` — presence rooms with join, leave, presence updates,
//! topic broadcast, listing, and per-room member queries.
//!
//! All endpoints require_auth. Caller identity is server-resolved from
//! the session — only admins may spoof another user_id via the body
//! (used for server-to-server presence mirroring).

use crate::{json_error, json_error_safe, require_auth, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    if url == "/api/rooms/join" && method == HttpMethod::Post {
        if let Some(err) = require_auth(ctx) {
            return Some(err);
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
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return Some((400, json_error("MISSING_ROOM", "room is required"))),
        };
        let body_user = data.get("user_id").and_then(|v| v.as_str());
        let user_id = if ctx.auth_ctx.is_admin {
            body_user.or(ctx.auth_ctx.user_id.as_deref())
        } else {
            ctx.auth_ctx.user_id.as_deref()
        };
        let user_id = match user_id {
            Some(u) => u,
            None => {
                return Some((
                    401,
                    json_error("AUTH_REQUIRED", "authenticated session required"),
                ));
            }
        };
        let user_data = data.get("data").cloned();

        let (snapshot, join_event) = match ctx.rooms.join(room, user_id, user_data) {
            Ok(result) => result,
            Err(e) => return Some((429, json_error(&e.code, &e.message))),
        };

        if let Ok(json) = serde_json::to_string(&join_event) {
            ctx.notifier.notify_presence(&json);
        }

        return Some((
            200,
            serde_json::json!({
                "joined": room,
                "snapshot": snapshot,
            })
            .to_string(),
        ));
    }

    if url == "/api/rooms/leave" && method == HttpMethod::Post {
        if let Some(err) = require_auth(ctx) {
            return Some(err);
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
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return Some((400, json_error("MISSING_ROOM", "room is required"))),
        };
        let body_user = data.get("user_id").and_then(|v| v.as_str());
        let user_id = if ctx.auth_ctx.is_admin {
            body_user.or(ctx.auth_ctx.user_id.as_deref())
        } else {
            ctx.auth_ctx.user_id.as_deref()
        };
        let user_id = match user_id {
            Some(u) => u,
            None => {
                return Some((
                    401,
                    json_error("AUTH_REQUIRED", "authenticated session required"),
                ));
            }
        };

        // Idempotent: leaving a room you weren't in returns
        // `was_present: false` instead of 404.
        if let Some(leave_event) = ctx.rooms.leave(room, user_id) {
            if let Ok(json) = serde_json::to_string(&leave_event) {
                ctx.notifier.notify_presence(&json);
            }
            return Some((
                200,
                serde_json::json!({"left": room, "was_present": true}).to_string(),
            ));
        }
        return Some((
            200,
            serde_json::json!({"left": room, "was_present": false}).to_string(),
        ));
    }

    if url == "/api/rooms/presence" && method == HttpMethod::Post {
        if let Some(err) = require_auth(ctx) {
            return Some(err);
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
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return Some((400, json_error("MISSING_ROOM", "room is required"))),
        };
        let body_user = data.get("user_id").and_then(|v| v.as_str());
        let user_id = if ctx.auth_ctx.is_admin {
            body_user.or(ctx.auth_ctx.user_id.as_deref())
        } else {
            ctx.auth_ctx.user_id.as_deref()
        };
        let user_id = match user_id {
            Some(u) => u,
            None => {
                return Some((
                    401,
                    json_error("AUTH_REQUIRED", "authenticated session required"),
                ));
            }
        };
        let presence_data = data.get("data").cloned().unwrap_or(serde_json::json!({}));

        if let Some(presence_event) = ctx.rooms.set_presence(room, user_id, presence_data) {
            if let Ok(json) = serde_json::to_string(&presence_event) {
                ctx.notifier.notify_presence(&json);
            }
            return Some((200, serde_json::json!({"updated": true}).to_string()));
        }
        return Some((
            200,
            serde_json::json!({"updated": false, "reason": "not_in_room"}).to_string(),
        ));
    }

    if url == "/api/rooms/broadcast" && method == HttpMethod::Post {
        if let Some(err) = require_auth(ctx) {
            return Some(err);
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
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return Some((400, json_error("MISSING_ROOM", "room is required"))),
        };
        let topic = match data.get("topic").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return Some((400, json_error("MISSING_TOPIC", "topic is required"))),
        };
        let body_sender = data.get("user_id").and_then(|v| v.as_str());
        let sender = if ctx.auth_ctx.is_admin {
            body_sender.or(ctx.auth_ctx.user_id.as_deref())
        } else {
            ctx.auth_ctx.user_id.as_deref()
        };
        let broadcast_data = data.get("data").cloned().unwrap_or(serde_json::json!({}));

        if let Some(broadcast_event) = ctx.rooms.broadcast(room, sender, topic, broadcast_data) {
            if let Ok(json) = serde_json::to_string(&broadcast_event) {
                ctx.notifier.notify_presence(&json);
            }
            return Some((200, serde_json::json!({"broadcasted": true}).to_string()));
        }
        return Some((
            200,
            serde_json::json!({"broadcasted": false, "reason": "room_empty"}).to_string(),
        ));
    }

    if url == "/api/rooms" && method == HttpMethod::Get {
        if let Some(err) = require_auth(ctx) {
            return Some(err);
        }
        let room_names = ctx.rooms.list_rooms();
        let rooms: Vec<serde_json::Value> = room_names
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "members": ctx.rooms.room_size(name),
                })
            })
            .collect();
        return Some((
            200,
            serde_json::to_string(&rooms).unwrap_or_else(|_| "[]".into()),
        ));
    }

    if let Some(room_name) = url.strip_prefix("/api/rooms/") {
        let room_name = room_name.split('?').next().unwrap_or(room_name);
        if method == HttpMethod::Get
            && room_name != "join"
            && room_name != "leave"
            && room_name != "presence"
            && room_name != "broadcast"
        {
            if let Some(err) = require_auth(ctx) {
                return Some(err);
            }
            let members = ctx.rooms.members(room_name);
            return Some((
                200,
                serde_json::json!({
                    "room": room_name,
                    "members": members,
                    "count": members.len(),
                })
                .to_string(),
            ));
        }
    }

    None
}
