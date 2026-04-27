//! Workflow engine endpoints. All admin-gated — workflow instances
//! carry `input`, step `outputs`, `errors`, and execution status, all
//! sensitive operational data. Apps that surface workflow state to
//! end users do it through server functions with explicit policy.

use crate::{json_error, json_error_safe, require_admin, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    if url == "/api/workflows/definitions" && method == HttpMethod::Get {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        let defs = ctx.workflows.definitions();
        return Some((
            200,
            serde_json::to_string(&defs).unwrap_or_else(|_| "[]".into()),
        ));
    }

    if url == "/api/workflows/start" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
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
        let name = match data.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => return Some((400, json_error("MISSING_FIELD", "\"name\" is required"))),
        };
        let input = data.get("input").cloned().unwrap_or(serde_json::json!({}));
        return Some(match ctx.workflows.start(&name, input) {
            Ok(id) => (201, serde_json::json!({"id": id}).to_string()),
            Err(e) => (400, json_error("WORKFLOW_START_FAILED", &e)),
        });
    }

    // GET /api/workflows (list, status filter)
    if url.starts_with("/api/workflows")
        && !url.starts_with("/api/workflows/")
        && method == HttpMethod::Get
    {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        let status_filter = url
            .split("status=")
            .nth(1)
            .and_then(|s| s.split('&').next());
        let instances = ctx.workflows.list(status_filter);
        return Some((
            200,
            serde_json::to_string(&instances).unwrap_or_else(|_| "[]".into()),
        ));
    }

    // /api/workflows/<id> + sub-actions
    if let Some(rest) = url.strip_prefix("/api/workflows/") {
        let rest = rest.split('?').next().unwrap_or(rest);
        let (wf_id, sub) = match rest.find('/') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };

        if !wf_id.is_empty() && !wf_id.starts_with("definitions") {
            match (method, sub) {
                (HttpMethod::Get, None) => {
                    if let Some(err) = require_admin(ctx) {
                        return Some(err);
                    }
                    return Some(match ctx.workflows.get(wf_id) {
                        Some(inst) => (
                            200,
                            serde_json::to_string(&inst).unwrap_or_else(|_| "{}".into()),
                        ),
                        None => (
                            404,
                            json_error("NOT_FOUND", &format!("Workflow {wf_id} not found")),
                        ),
                    });
                }
                (HttpMethod::Post, Some("advance")) => {
                    if let Some(err) = require_admin(ctx) {
                        return Some(err);
                    }
                    return Some(match ctx.workflows.advance(wf_id) {
                        Ok(status) => (200, serde_json::json!({"status": status}).to_string()),
                        Err(e) => (400, json_error("WORKFLOW_ADVANCE_FAILED", &e)),
                    });
                }
                (HttpMethod::Post, Some("event")) => {
                    if let Some(err) = require_admin(ctx) {
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
                    let event = match data.get("event").and_then(|v| v.as_str()) {
                        Some(e) => e.to_string(),
                        None => {
                            return Some((
                                400,
                                json_error("MISSING_FIELD", "\"event\" is required"),
                            ))
                        }
                    };
                    let event_data = data.get("data").cloned().unwrap_or(serde_json::json!({}));
                    return Some(match ctx.workflows.send_event(wf_id, &event, event_data) {
                        Ok(()) => (200, serde_json::json!({"ok": true}).to_string()),
                        Err(e) => (400, json_error("WORKFLOW_EVENT_FAILED", &e)),
                    });
                }
                (HttpMethod::Post, Some("cancel")) => {
                    if let Some(err) = require_admin(ctx) {
                        return Some(err);
                    }
                    return Some(match ctx.workflows.cancel(wf_id) {
                        Ok(()) => (200, serde_json::json!({"cancelled": true}).to_string()),
                        Err(e) => (400, json_error("WORKFLOW_CANCEL_FAILED", &e)),
                    });
                }
                _ => {}
            }
        }
    }

    None
}
