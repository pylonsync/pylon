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
            // recent uploads. Require any authenticated identity here.
            if let Some(err) = require_auth(ctx) {
                return Some(err);
            }
            let (s, b) = ctx.files.get_file(file_id);
            return Some((s, b));
        }
    }

    None
}
