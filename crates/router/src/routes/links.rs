//! `/api/link` and `/api/unlink` — set / clear a foreign-key relation
//! on a row. Treated as a write (calls `check_entity_write` against
//! the source entity).

use crate::{json_error, json_error_safe, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    if url == "/api/link" && method == HttpMethod::Post {
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
        let entity = data.get("entity").and_then(|v| v.as_str()).unwrap_or("");
        let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let relation = data.get("relation").and_then(|v| v.as_str()).unwrap_or("");
        let target_id = data.get("target_id").and_then(|v| v.as_str()).unwrap_or("");

        // A link is a mutation: it sets a foreign key on the source row.
        // Apply the same write policy as PATCH /api/entities/:name/:id.
        let check = ctx
            .policy_engine
            .check_entity_write(entity, ctx.auth_ctx, Some(&data));
        if let pylon_policy::PolicyResult::Denied {
            policy_name,
            reason,
        } = check
        {
            tracing::warn!("[policy] link on {entity} denied by \"{policy_name}\": {reason}");
            return Some((403, json_error("POLICY_DENIED", "Access denied by policy")));
        }

        return Some(match ctx.store.link(entity, id, relation, target_id) {
            Ok(true) => (200, serde_json::json!({"linked": true}).to_string()),
            Ok(false) => (404, json_error("NOT_FOUND", "Source entity not found")),
            Err(e) => (400, json_error(&e.code, &e.message)),
        });
    }

    if url == "/api/unlink" && method == HttpMethod::Post {
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
        let entity = data.get("entity").and_then(|v| v.as_str()).unwrap_or("");
        let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let relation = data.get("relation").and_then(|v| v.as_str()).unwrap_or("");

        let check = ctx
            .policy_engine
            .check_entity_write(entity, ctx.auth_ctx, Some(&data));
        if let pylon_policy::PolicyResult::Denied {
            policy_name,
            reason,
        } = check
        {
            tracing::warn!("[policy] unlink on {entity} denied by \"{policy_name}\": {reason}");
            return Some((403, json_error("POLICY_DENIED", "Access denied by policy")));
        }

        return Some(match ctx.store.unlink(entity, id, relation) {
            Ok(true) => (200, serde_json::json!({"unlinked": true}).to_string()),
            Ok(false) => (404, json_error("NOT_FOUND", "Source entity not found")),
            Err(e) => (400, json_error(&e.code, &e.message)),
        });
    }

    None
}
