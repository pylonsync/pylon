//! Cloudflare R2-backed implementation of `FileOps`.
//!
//! Only built with the `workers` feature (wasm32 target). The
//! non-WASM build uses `NoopAll`'s stub Files which returns 503
//! R2_BINDING_REQUIRED.
//!
//! # Upload path
//!
//! The trait's `upload(body: &str)` method is intentionally
//! limited — the runtime crate's adapter also rejects string-body
//! uploads because binary file payloads can't round-trip through
//! a `String` losslessly. On Workers, binary uploads must be
//! handled in the fetch handler BEFORE the body is coerced to a
//! String — see `handler.rs` for the multipart/raw-binary
//! routing.
//!
//! This adapter implements `upload` defensively (same shape as
//! the runtime) and the read path (`get_file`) end-to-end against
//! R2.

#![cfg(feature = "workers")]

use futures::executor::block_on;
use pylon_router::FileOps;
use worker::Bucket;

/// FileOps backed by a Cloudflare R2 bucket binding.
///
/// Construct from `env.bucket("PYLON_FILES")` in the fetch handler.
pub struct R2Files {
    bucket: Bucket,
}

impl R2Files {
    pub fn new(bucket: Bucket) -> Self {
        Self { bucket }
    }

    /// Upload binary bytes directly to R2 under the supplied key.
    /// Called by the fetch handler's multipart / raw-binary
    /// routing — not exposed via the trait surface because the
    /// trait works in `&str` body terms (lossy for binary).
    ///
    /// Returns the same shape the self-hosted runtime returns for
    /// successful uploads: `{ ok: true, id: <key>, size: <n> }`.
    pub fn upload_binary(&self, id: &str, bytes: &[u8]) -> (u16, String) {
        let put_builder = self.bucket.put(id, bytes.to_vec());
        match block_on(put_builder.execute()) {
            Ok(_) => (
                200,
                serde_json::json!({
                    "ok": true,
                    "id": id,
                    "size": bytes.len(),
                })
                .to_string(),
            ),
            Err(e) => (
                500,
                pylon_router::json_error("R2_PUT_FAILED", &format!("R2 put {id}: {e}")),
            ),
        }
    }
}

impl FileOps for R2Files {
    fn upload(&self, _body: &str) -> (u16, String) {
        // Same defensive posture as the self-hosted runtime — binary
        // uploads can't go through a String body. handler.rs
        // intercepts /api/files/upload before the body is coerced
        // and routes to `upload_binary` directly.
        (
            400,
            pylon_router::json_error(
                "UPLOAD_NEEDS_BINARY",
                "File uploads must use multipart/form-data or raw binary with X-Filename; the trait's string-body path is for defensive rejection only.",
            ),
        )
    }

    fn get_file(
        &self,
        id: &str,
        _requester_user_id: Option<&str>,
        _is_admin: bool,
    ) -> (u16, String) {
        // R2 doesn't track per-file ownership the way LocalFileStorage
        // does; ownership / read auth is mediated by whatever auth
        // gate the route layer applied upstream. The trait's
        // `requester_user_id` + `is_admin` params are ignored here
        // for the same reason Stack0's FileOps impl ignores them
        // (its bucket access is also unmediated).
        //
        // Apps that want per-file ACLs on R2 should layer them
        // explicitly via a separate metadata store and check
        // them in a plugin's `before_file_get` hook.
        let obj = match block_on(self.bucket.get(id).execute()) {
            Ok(Some(o)) => o,
            Ok(None) => {
                return (
                    404,
                    pylon_router::json_error("FILE_NOT_FOUND", "File not found"),
                );
            }
            Err(e) => {
                return (
                    500,
                    pylon_router::json_error("R2_GET_FAILED", &format!("R2 get {id}: {e}")),
                );
            }
        };
        let body = match obj.body() {
            Some(b) => b,
            None => {
                return (
                    500,
                    pylon_router::json_error(
                        "R2_EMPTY_BODY",
                        "R2 returned the object metadata with no body bytes",
                    ),
                );
            }
        };
        match block_on(body.bytes()) {
            Ok(bytes) => {
                // FileOps returns the body as a String — same
                // wire-format limitation as the trait's upload.
                // Real binary downloads on Workers should route
                // through handler.rs which can stream R2's
                // ReadableStream directly to the response.
                (200, String::from_utf8_lossy(&bytes).into_owned())
            }
            Err(e) => (
                500,
                pylon_router::json_error("R2_READ_FAILED", &format!("R2 read {id}: {e}")),
            ),
        }
    }
}
