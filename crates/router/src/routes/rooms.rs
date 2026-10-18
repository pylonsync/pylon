//! `/api/rooms/*` — presence rooms with join, leave, presence updates,
//! heartbeats, topic broadcast, listing, and per-room member queries.
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
        if let Err(denied) = room_access(ctx, room) {
            return Some(denied.response());
        }
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
        if let Err(denied) = room_access(ctx, room) {
            return Some(denied.response());
        }

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

    // Keepalive. Members idle past the runtime's timeout are removed by
    // the idle sweep; a heartbeat marks the member active with no event.
    // `alive: false` tells the client it is no longer in the room and
    // must rejoin.
    if url == "/api/rooms/heartbeat" && method == HttpMethod::Post {
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

        // Access lost since joining (unshared, removed from the workspace):
        // the member leaves, which also ends their subscriptions to the room.
        if let Err(denied) = room_access(ctx, room) {
            if let Some(leave_event) = ctx.rooms.leave(room, user_id) {
                let mut event = leave_event;
                if let Some(obj) = event.as_object_mut() {
                    obj.insert("revoked".into(), serde_json::Value::Bool(true));
                }
                if let Ok(json) = serde_json::to_string(&event) {
                    ctx.notifier.notify_presence(&json);
                }
            }
            return Some((
                200,
                serde_json::json!({"alive": false, "reason": denied.code()}).to_string(),
            ));
        }
        if ctx.rooms.touch(room, user_id) {
            return Some((200, serde_json::json!({"alive": true}).to_string()));
        }
        return Some((
            200,
            serde_json::json!({"alive": false, "reason": "not_in_room"}).to_string(),
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
        if let Err(denied) = room_access(ctx, room) {
            return Some(denied.response());
        }

        // Auth gate (mirrors set_presence above): require an
        // identifiable sender, and require non-admin senders to be
        // members of the target room. Without this, any authed user
        // could blast topics into rooms they never joined. Admins
        // bypass for server-to-server use cases; the runtime layer
        // (RoomManager::broadcast) still enforces membership when an
        // admin tries to spoof a non-member sender — defense in depth.
        let sender_id = match sender {
            Some(s) => s,
            None => {
                return Some((
                    401,
                    json_error("AUTH_REQUIRED", "authenticated session required"),
                ));
            }
        };
        if !ctx.auth_ctx.is_admin && !ctx.rooms.is_in_room(room, sender_id) {
            // Mirror set_presence's response shape (`not_in_room`) so
            // clients can branch on one consistent gating reason
            // across presence + broadcast. Do not call into the runtime
            // — the membership check has already failed.
            return Some((
                200,
                serde_json::json!({"broadcasted": false, "reason": "not_in_room"}).to_string(),
            ));
        }

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
        // A room tied to a row names the row: list it only to callers who
        // may read the row.
        let rooms: Vec<serde_json::Value> = room_names
            .iter()
            .filter(|name| room_access(ctx, name).is_ok())
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
            && room_name != "heartbeat"
            && room_name != "broadcast"
        {
            if let Some(err) = require_auth(ctx) {
                return Some(err);
            }
            let room_name = crate::url_decode(room_name);
            if let Err(denied) = room_access(ctx, &room_name) {
                return Some(denied.response());
            }
            let members = ctx.rooms.members(&room_name);
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

/// The row a room is tied to: a room named `@Entity/rowId` (built by
/// `useRoom({ entity, id })`). Its members are the people who may read the
/// row. Any other name is a free room.
pub fn row_room(room: &str) -> Option<(&str, &str)> {
    let rest = room.strip_prefix('@')?;
    let (entity, id) = rest.split_once('/')?;
    if entity.is_empty() || id.is_empty() {
        return None;
    }
    Some((entity, id))
}

/// Why a caller may not use a room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomDenied {
    /// A name starting with `@` that does not name a row. `@` is reserved
    /// for rooms tied to a row, so a typo cannot make a free room.
    InvalidName,
    /// The row does not exist, or the caller may not read it. One answer for
    /// both, so a caller cannot probe for rows.
    Forbidden,
}

impl RoomDenied {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidName => "INVALID_ROOM",
            Self::Forbidden => "ROOM_FORBIDDEN",
        }
    }

    fn response(&self) -> (u16, String) {
        match self {
            Self::InvalidName => (
                400,
                json_error(
                    "INVALID_ROOM",
                    "A room name starting with @ must be @Entity/rowId",
                ),
            ),
            Self::Forbidden => (
                403,
                json_error("ROOM_FORBIDDEN", "You cannot use this room"),
            ),
        }
    }
}

/// Whether `auth` may use `room`: free rooms are open to any signed-in
/// caller, as before; a room tied to a row needs the row's read policy.
/// Every room operation (join, presence, heartbeat, messages, the member
/// list, the WebSocket subscription) runs this, so access lost after joining
/// takes effect at the next one.
pub fn check_room_access(
    store: &dyn pylon_http::DataStore,
    policy: &pylon_policy::PolicyEngine,
    auth: &pylon_auth::AuthContext,
    room: &str,
) -> Result<(), RoomDenied> {
    if !room.starts_with('@') {
        return Ok(());
    }
    let (entity, id) = row_room(room).ok_or(RoomDenied::InvalidName)?;
    if !store.manifest().entities.iter().any(|e| e.name == entity) {
        return Err(RoomDenied::Forbidden);
    }
    let row = match store.get_by_id(entity, id) {
        Ok(Some(row)) => row,
        _ => return Err(RoomDenied::Forbidden),
    };
    match policy.check_entity_read(entity, auth, Some(&row)) {
        pylon_policy::PolicyResult::Allowed => Ok(()),
        pylon_policy::PolicyResult::Denied { .. } => Err(RoomDenied::Forbidden),
    }
}

fn room_access(ctx: &RouterContext, room: &str) -> Result<(), RoomDenied> {
    check_room_access(ctx.store, ctx.policy_engine, ctx.auth_ctx, room)
}

#[cfg(test)]
mod tests {
    use super::row_room;

    #[test]
    fn row_rooms_name_an_entity_and_an_id() {
        assert_eq!(row_room("@Document/abc"), Some(("Document", "abc")));
        assert_eq!(row_room("@Document/a/b"), Some(("Document", "a/b")));
        assert_eq!(row_room("doc:abc"), None);
        assert_eq!(row_room("@Document"), None);
        assert_eq!(row_room("@/abc"), None);
        assert_eq!(row_room("@Document/"), None);
    }
}
