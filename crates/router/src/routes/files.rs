//! `/api/files/upload` (POST, opaque body to FileOps) and
//! `/api/files/<id>` (GET, requires_auth — file IDs are predictable).

use crate::{require_auth, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    if url == "/api/files/upload" && method == HttpMethod::Post {
        let (s, b) = ctx.files.upload(body);
        return Some((s, b));
    }

    if let Some(file_id) = url.strip_prefix("/api/files/") {
        let file_id = file_id.split('?').next().unwrap_or(file_id);
        if method == HttpMethod::Get {
            // File IDs are timestamp + sanitised filename — predictable
            // enough that an unauthenticated caller could enumerate
            // recent uploads. Require an authenticated identity AND a
            // matching owner record (enforced inside `get_file`).
            //
            // Pass `is_unscoped_admin()`, not bare `is_admin`: an admin
            // acting WITHIN a tenant context (is_admin + active tenant)
            // must stay scoped to the owner check, matching the #354/#355
            // access-control hardening — only an admin with NO active
            // tenant gets the cross-owner read bypass.
            if let Some(err) = require_auth(ctx) {
                return Some(err);
            }
            let (s, b) = ctx.files.get_file(
                file_id,
                ctx.auth_ctx.user_id.as_deref(),
                ctx.auth_ctx.is_unscoped_admin(),
            );
            return Some((s, b));
        }
    }

    None
}
