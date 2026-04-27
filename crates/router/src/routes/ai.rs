//! `/api/ai/*` — AI completion shim. The router-level handler always
//! returns 503; the streaming variant lives in the runtime layer
//! (`server.rs`) since it needs streaming I/O the platform-agnostic
//! router can't model.

use crate::{json_error, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    _ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    _body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    if url == "/api/ai/complete" && method == HttpMethod::Post {
        return Some((
            503,
            json_error(
                "AI_NOT_CONFIGURED",
                "AI completion is not available on this platform",
            ),
        ));
    }
    None
}
