//! `/api/link` and `/api/unlink` — set / clear a foreign-key relation
//! on a row. Authorized as an Update (changing the FK column on the
//! existing row), so policy and plugin hooks behave exactly like
//! `PATCH /api/entities/<entity>/<id>` would.

use crate::mutate::{apply_mutation, error_response, MutationCtx, MutationError, MutationOp};
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
        let mctx = MutationCtx::from_router(ctx);
        return Some(
            match apply_mutation(
                &mctx,
                MutationOp::Link {
                    entity,
                    row_id: id,
                    relation,
                    target_id,
                },
            ) {
                Ok(_outcome) => (200, serde_json::json!({"linked": true}).to_string()),
                Err(MutationError::NotFound) => {
                    (404, json_error("NOT_FOUND", &format!("{entity}/{id} not found")))
                }
                Err(err) => {
                    let (status, code, message) = error_response(&err);
                    (status, json_error(&code, &message))
                }
            },
        );
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
        let mctx = MutationCtx::from_router(ctx);
        return Some(
            match apply_mutation(
                &mctx,
                MutationOp::Unlink {
                    entity,
                    row_id: id,
                    relation,
                },
            ) {
                Ok(_outcome) => (200, serde_json::json!({"unlinked": true}).to_string()),
                Err(MutationError::NotFound) => {
                    (404, json_error("NOT_FOUND", &format!("{entity}/{id} not found")))
                }
                Err(err) => {
                    let (status, code, message) = error_response(&err);
                    (status, json_error(&code, &message))
                }
            },
        );
    }

    None
}
