//! Platform-agnostic HTTP router for pylon.
//!
//! This crate contains the pure routing logic that maps HTTP requests to
//! data store operations. It has no I/O dependencies — no `tiny_http`,
//! no `tungstenite`, no `rusqlite`. It works with any [`DataStore`]
//! implementation (SQLite Runtime, Cloudflare D1, etc.).

use pylon_auth::{AuthContext, CookieConfig, MagicCodeStore, OAuthStateStore, SessionStore};
use pylon_http::{DataError, DataStore, HttpMethod};
use pylon_policy::PolicyEngine;
use pylon_sync::{ChangeKind, ChangeLog};
use std::cell::RefCell;

pub mod merge;
mod routes;

// ---------------------------------------------------------------------------
// ChangeNotifier — abstraction over WS/SSE broadcast
// ---------------------------------------------------------------------------

/// Receives change notifications for real-time sync.
///
/// On the self-hosted server this broadcasts to WebSocket + SSE hubs.
/// On Workers this can be a no-op or post to a Durable Object.
pub trait ChangeNotifier: Send + Sync {
    fn notify(&self, event: &pylon_sync::ChangeEvent);
    fn notify_presence(&self, json: &str);

    /// Ship a binary CRDT update for one row to subscribed clients.
    /// Called after every successful write to a CRDT-mode entity. The
    /// payload is a Loro snapshot (or eventually an incremental delta);
    /// the implementation owns wire-format framing — see
    /// `encode_crdt_frame` for the canonical Pylon shape.
    ///
    /// Default impl is a no-op so backends without WebSocket support
    /// (Workers, no-op notifier) compile without ceremony.
    fn notify_crdt(&self, _entity: &str, _row_id: &str, _snapshot: &[u8]) {}

    /// Push a `session-changed` envelope to every connected WS client
    /// whose authenticated user_id matches. Fires when the server
    /// mutates a session (select-org, clear-org, revoke). The SDK's
    /// SyncEngine handles the envelope by refetching /api/auth/me and
    /// resetting the local replica if the visible set changed.
    ///
    /// `tenant_id` is the post-mutation value (None for "no active
    /// org" / "revoked"). The SDK doesn't actually trust this scalar
    /// — it re-reads /api/auth/me — but ships it so consumers that
    /// want low-latency tenant readout can use it without a round
    /// trip. Default impl is a no-op for non-WS backends.
    fn notify_session_changed(&self, _user_id: &str, _tenant_id: Option<&str>) {}
}

/// No-op notifier for platforms without real-time push.
pub struct NoopNotifier;

impl ChangeNotifier for NoopNotifier {
    fn notify(&self, _event: &pylon_sync::ChangeEvent) {}
    fn notify_presence(&self, _json: &str) {}
}

// ---------------------------------------------------------------------------
// CRDT wire format
//
// Every CRDT broadcast frame is a single binary WebSocket message shaped:
//
//   [type: u8] [entity_len: u16 BE] [entity utf8] [row_id_len: u16 BE] [row_id utf8] [payload bytes]
//
// Type bytes (matching the Remboard pattern that proved out in production):
//
//   0x10 = full Loro snapshot (sent on subscribe / first writes)
//   0x11 = incremental Loro update (sent on subsequent writes)
//
// For the first slice we always send 0x10 — Loro's snapshots are bounded
// by internal compaction so the bandwidth is fine; switching to deltas
// is a non-breaking optimization (just flip the type byte and the
// payload encoding) once we have per-client version-vector tracking.
// ---------------------------------------------------------------------------

/// Frame type for a full CRDT snapshot.
pub const CRDT_FRAME_SNAPSHOT: u8 = 0x10;
/// Frame type for an incremental CRDT update (reserved — not yet emitted).
pub const CRDT_FRAME_UPDATE: u8 = 0x11;

/// Errors from [`encode_crdt_frame`]. Surfaced loud rather than silently
/// truncating so a pathological entity / row_id name (>64 KiB) becomes
/// an observable failure instead of a malformed frame the client can't
/// decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameEncodeError {
    /// Entity name exceeds the 16-bit length header. In practice every
    /// Pylon entity name is well under 100 bytes — hitting this means
    /// the caller is using the encoder for something it wasn't designed
    /// for. The bound is `u16::MAX = 65535` bytes (UTF-8 length).
    EntityTooLong { len: usize },
    /// Row ID exceeds the 16-bit length header. Pylon-generated IDs are
    /// 40 hex chars; user-supplied IDs aren't validated up to this layer
    /// but are practically bounded by URL / SQL constraints elsewhere.
    RowIdTooLong { len: usize },
}

impl std::fmt::Display for FrameEncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EntityTooLong { len } => write!(
                f,
                "CRDT frame: entity name {len} bytes exceeds u16 length limit ({})",
                u16::MAX
            ),
            Self::RowIdTooLong { len } => write!(
                f,
                "CRDT frame: row_id {len} bytes exceeds u16 length limit ({})",
                u16::MAX
            ),
        }
    }
}

impl std::error::Error for FrameEncodeError {}

/// Encode a CRDT broadcast frame. Layout documented at the top of this
/// module. Returns the encoded bytes on success; errors when entity /
/// row_id can't fit the 16-bit length header (~65 KiB, never hit in
/// practice).
///
/// Client decoders mirror this in `@pylonsync/loro/src/wire.ts`.
pub fn encode_crdt_frame(
    frame_type: u8,
    entity: &str,
    row_id: &str,
    payload: &[u8],
) -> Result<Vec<u8>, FrameEncodeError> {
    let entity_bytes = entity.as_bytes();
    let row_id_bytes = row_id.as_bytes();
    if entity_bytes.len() > u16::MAX as usize {
        return Err(FrameEncodeError::EntityTooLong {
            len: entity_bytes.len(),
        });
    }
    if row_id_bytes.len() > u16::MAX as usize {
        return Err(FrameEncodeError::RowIdTooLong {
            len: row_id_bytes.len(),
        });
    }
    let entity_len = entity_bytes.len() as u16;
    let row_id_len = row_id_bytes.len() as u16;
    let mut out =
        Vec::with_capacity(1 + 2 + entity_bytes.len() + 2 + row_id_bytes.len() + payload.len());
    out.push(frame_type);
    out.extend_from_slice(&entity_len.to_be_bytes());
    out.extend_from_slice(entity_bytes);
    out.extend_from_slice(&row_id_len.to_be_bytes());
    out.extend_from_slice(row_id_bytes);
    out.extend_from_slice(payload);
    Ok(out)
}

#[cfg(test)]
mod crdt_frame_tests {
    use super::*;

    #[test]
    fn roundtrip_header_layout() {
        let frame = encode_crdt_frame(
            CRDT_FRAME_SNAPSHOT,
            "Message",
            "msg_123",
            &[0xab, 0xcd, 0xef],
        )
        .unwrap();
        assert_eq!(frame[0], 0x10);
        // entity_len = 7 ("Message") in BE
        assert_eq!(&frame[1..3], &[0, 7]);
        assert_eq!(&frame[3..10], b"Message");
        // row_id_len = 7 ("msg_123") in BE
        assert_eq!(&frame[10..12], &[0, 7]);
        assert_eq!(&frame[12..19], b"msg_123");
        // payload trails after both headers
        assert_eq!(&frame[19..], &[0xab, 0xcd, 0xef]);
    }

    #[test]
    fn empty_payload_still_carries_headers() {
        let frame = encode_crdt_frame(CRDT_FRAME_UPDATE, "X", "y", &[]).unwrap();
        assert_eq!(frame[0], 0x11);
        assert_eq!(&frame[1..3], &[0, 1]);
        assert_eq!(&frame[3..4], b"X");
        assert_eq!(&frame[4..6], &[0, 1]);
        assert_eq!(&frame[6..7], b"y");
        assert_eq!(frame.len(), 7);
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(super::decode_hex(""), Some(vec![]));
        assert_eq!(super::decode_hex("00"), Some(vec![0x00]));
        assert_eq!(super::decode_hex("ab"), Some(vec![0xab]));
        assert_eq!(super::decode_hex("AB"), Some(vec![0xab]));
        assert_eq!(
            super::decode_hex("DEADBEEF"),
            Some(vec![0xde, 0xad, 0xbe, 0xef])
        );
    }

    #[test]
    fn hex_rejects_malformed() {
        assert_eq!(super::decode_hex("a"), None); // odd length
        assert_eq!(super::decode_hex("xy"), None); // non-hex
        assert_eq!(super::decode_hex("ab cd"), None); // space inside
    }

    #[test]
    fn entity_too_long_errors() {
        let huge_entity = "x".repeat(u16::MAX as usize + 1);
        let err = encode_crdt_frame(CRDT_FRAME_SNAPSHOT, &huge_entity, "y", &[])
            .expect_err("entity > u16::MAX must reject");
        assert!(matches!(err, FrameEncodeError::EntityTooLong { .. }));
    }

    #[test]
    fn row_id_too_long_errors() {
        let huge_row_id = "x".repeat(u16::MAX as usize + 1);
        let err = encode_crdt_frame(CRDT_FRAME_SNAPSHOT, "X", &huge_row_id, &[])
            .expect_err("row_id > u16::MAX must reject");
        assert!(matches!(err, FrameEncodeError::RowIdTooLong { .. }));
    }
}

// ---------------------------------------------------------------------------
// CacheOps / PubSubOps / JobOps / SchedulerOps / WorkflowOps
// — thin traits so the router doesn't depend on concrete impls
// ---------------------------------------------------------------------------

/// Cache operations used by the router.
pub trait CacheOps: Send + Sync {
    fn handle_command(&self, body: &str) -> (u16, String);
    fn handle_get(&self, key: &str) -> (u16, String);
    fn handle_delete(&self, key: &str) -> (u16, String);
}

/// Pub/Sub operations used by the router.
pub trait PubSubOps: Send + Sync {
    fn handle_publish(&self, body: &str) -> (u16, String);
    fn handle_channels(&self) -> (u16, String);
    fn handle_history(&self, channel: &str, url: &str) -> (u16, String);
}

/// Room operations used by the router.
pub trait RoomOps: Send + Sync {
    fn join(
        &self,
        room: &str,
        user_id: &str,
        data: Option<serde_json::Value>,
    ) -> Result<(serde_json::Value, serde_json::Value), DataError>;
    fn leave(&self, room: &str, user_id: &str) -> Option<serde_json::Value>;
    fn set_presence(
        &self,
        room: &str,
        user_id: &str,
        data: serde_json::Value,
    ) -> Option<serde_json::Value>;
    fn broadcast(
        &self,
        room: &str,
        sender: Option<&str>,
        topic: &str,
        data: serde_json::Value,
    ) -> Option<serde_json::Value>;
    fn list_rooms(&self) -> Vec<String>;
    fn room_size(&self, name: &str) -> usize;
    fn members(&self, name: &str) -> Vec<serde_json::Value>;
}

/// Job queue operations used by the router.
pub trait JobOps: Send + Sync {
    fn enqueue(
        &self,
        name: &str,
        payload: serde_json::Value,
        priority: &str,
        delay_secs: u64,
        max_retries: u32,
        queue: &str,
    ) -> String;
    fn stats(&self) -> serde_json::Value;
    fn dead_letters(&self) -> serde_json::Value;
    fn retry_dead(&self, id: &str) -> bool;
    fn list_jobs(
        &self,
        status: Option<&str>,
        queue: Option<&str>,
        limit: usize,
    ) -> serde_json::Value;
    fn get_job(&self, id: &str) -> Option<serde_json::Value>;
}

/// Scheduler operations used by the router.
pub trait SchedulerOps: Send + Sync {
    fn list_tasks(&self) -> serde_json::Value;
    fn trigger(&self, name: &str) -> bool;
}

/// Workflow engine operations used by the router.
pub trait WorkflowOps: Send + Sync {
    fn definitions(&self) -> serde_json::Value;
    fn start(&self, name: &str, input: serde_json::Value) -> Result<String, String>;
    fn list(&self, status_filter: Option<&str>) -> serde_json::Value;
    fn get(&self, id: &str) -> Option<serde_json::Value>;
    fn advance(&self, id: &str) -> Result<String, String>;
    fn send_event(&self, id: &str, event: &str, data: serde_json::Value) -> Result<(), String>;
    fn cancel(&self, id: &str) -> Result<(), String>;
}

/// File storage operations used by the router.
pub trait FileOps: Send + Sync {
    fn upload(&self, body: &str) -> (u16, String);
    /// Retrieve a file by ID, gated on owner identity.
    ///
    /// Implementations must:
    /// - Allow access when `is_admin` is true.
    /// - Allow access when an owner record exists and `requester_user_id`
    ///   matches the recorded `user_id`.
    /// - Reject (403) when an owner record exists and does not match.
    /// - Allow access when no owner record exists AND the storage backend
    ///   does not require owner checks (e.g., Stack0). For backends that
    ///   do require owner checks, a missing record is treated as legacy
    ///   data and access is allowed but logged — new uploads always carry
    ///   an owner record so this fallback only covers files predating the
    ///   ownership machinery.
    fn get_file(&self, id: &str, requester_user_id: Option<&str>, is_admin: bool) -> (u16, String);
}

/// Sends emails (magic codes, invitations, etc.).
pub trait EmailSender: Send + Sync {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String>;
}

/// Access to sharded real-time simulations (game matches, MMO zones, etc.).
pub trait ShardOps: Send + Sync {
    /// Look up an existing shard by ID.
    fn get_shard(&self, id: &str) -> Option<std::sync::Arc<dyn pylon_realtime::DynShard>>;

    /// List IDs of active shards.
    fn list_shards(&self) -> Vec<String>;

    /// Number of active shards.
    fn shard_count(&self) -> usize;
}

/// Generates the OpenAPI spec JSON string for the manifest.
pub trait OpenApiGenerator: Send + Sync {
    fn generate(&self, base_url: &str) -> String;
}

/// Plugin CRUD lifecycle hooks used by the router.
///
/// Wired up so that `POST/PATCH/DELETE /api/entities/...` triggers registered
/// plugin before_/after_ hooks (audit log, search indexing, webhooks,
/// versioning, validation, timestamps, slugify). Previously the router only
/// ran `on_request` + custom routes, silently bypassing CRUD hooks — which
/// meant security-relevant plugins (validation, audit_log) didn't apply to
/// the primary write path.
///
/// `before_insert/update` receive a mutable `data` so plugins can inject
/// fields (timestamps, slugs). A returned `Err` rejects the write with the
/// given status + message and no data is touched.
pub trait PluginHookOps: Send + Sync {
    fn before_insert(
        &self,
        entity: &str,
        data: &mut serde_json::Value,
        auth: &AuthContext,
    ) -> Result<(), (u16, String, String)>;

    fn after_insert(&self, entity: &str, id: &str, data: &serde_json::Value, auth: &AuthContext);

    fn before_update(
        &self,
        entity: &str,
        id: &str,
        data: &mut serde_json::Value,
        auth: &AuthContext,
    ) -> Result<(), (u16, String, String)>;

    fn after_update(&self, entity: &str, id: &str, data: &serde_json::Value, auth: &AuthContext);

    fn before_delete(
        &self,
        entity: &str,
        id: &str,
        auth: &AuthContext,
    ) -> Result<(), (u16, String, String)>;

    fn after_delete(&self, entity: &str, id: &str, auth: &AuthContext);
}

/// No-op plugin hooks for platforms or tests without a registry.
pub struct NoopPluginHooks;

impl PluginHookOps for NoopPluginHooks {
    fn before_insert(
        &self,
        _entity: &str,
        _data: &mut serde_json::Value,
        _auth: &AuthContext,
    ) -> Result<(), (u16, String, String)> {
        Ok(())
    }
    fn after_insert(
        &self,
        _entity: &str,
        _id: &str,
        _data: &serde_json::Value,
        _auth: &AuthContext,
    ) {
    }
    fn before_update(
        &self,
        _entity: &str,
        _id: &str,
        _data: &mut serde_json::Value,
        _auth: &AuthContext,
    ) -> Result<(), (u16, String, String)> {
        Ok(())
    }
    fn after_update(
        &self,
        _entity: &str,
        _id: &str,
        _data: &serde_json::Value,
        _auth: &AuthContext,
    ) {
    }
    fn before_delete(
        &self,
        _entity: &str,
        _id: &str,
        _auth: &AuthContext,
    ) -> Result<(), (u16, String, String)> {
        Ok(())
    }
    fn after_delete(&self, _entity: &str, _id: &str, _auth: &AuthContext) {}
}

/// TypeScript function operations used by the router.
///
/// Implementations manage transaction semantics: mutations run under the
/// write lock with BEGIN/COMMIT/ROLLBACK, queries use the read pool,
/// actions run without transactions.
pub trait FnOps: Send + Sync {
    /// Look up a registered function.
    fn get_fn(&self, name: &str) -> Option<pylon_functions::registry::FnDef>;

    /// List all registered functions.
    fn list_fns(&self) -> Vec<pylon_functions::registry::FnDef>;

    /// Execute a function. For streaming responses, `on_stream` is called for
    /// each chunk as it arrives from the function handler.
    ///
    /// `request` is populated when the function is invoked via a custom HTTP
    /// route (`defineRoute` binding). It carries the raw request metadata
    /// (method, path, headers, body bytes) so actions can verify webhook
    /// signatures. Pass `None` for programmatic invocations (runAction,
    /// scheduled jobs, admin dashboard).
    fn call(
        &self,
        fn_name: &str,
        args: serde_json::Value,
        auth: pylon_functions::protocol::AuthInfo,
        on_stream: Option<Box<dyn FnMut(&str) + Send>>,
        request: Option<pylon_functions::protocol::RequestInfo>,
    ) -> Result<
        (serde_json::Value, pylon_functions::trace::FnTrace),
        pylon_functions::runner::FnCallError,
    >;

    /// Recent traces for observability (newest first).
    fn recent_traces(&self, limit: usize) -> Vec<pylon_functions::trace::FnTrace>;

    /// Check whether the caller is allowed to invoke this function right now.
    ///
    /// Returns `Ok(())` if allowed, or `Err(retry_after_secs)` if the caller
    /// is over the per-function quota. Default impl is permissive — backends
    /// that don't enforce per-function limits don't need to implement it.
    /// `identity` is a stable string for the caller (user id, session id, or
    /// IP) used as the rate-limit key.
    fn check_rate_limit(&self, _fn_name: &str, _identity: &str) -> Result<(), u64> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// RouterContext — bundles all dependencies for a single request
// ---------------------------------------------------------------------------

pub struct RouterContext<'a> {
    pub store: &'a dyn DataStore,
    pub session_store: &'a SessionStore,
    pub magic_codes: &'a MagicCodeStore,
    pub oauth_state: &'a OAuthStateStore,
    /// Persistent OAuth account links — better-auth's `account` table
    /// equivalent. Used by the OAuth callback to look up + upsert the
    /// `(provider, provider_account_id) → user_id` mapping plus the
    /// access/refresh token bundle.
    pub account_store: &'a pylon_auth::AccountStore,
    /// Long-lived API keys — `pk.key_<id>.<secret>` bearer tokens that
    /// resolve to a user_id with optional scopes/expiry. Created via
    /// `POST /api/auth/api-keys`, listed/revoked from the same path.
    /// Gated on wasm32 — `api_key` module pulls native crypto.
    #[cfg(not(target_arch = "wasm32"))]
    pub api_keys: &'a pylon_auth::api_key::ApiKeyStore,
    /// Organizations + memberships + invites — multi-tenant team
    /// management. Endpoints under `/api/auth/orgs/...`.
    pub orgs: &'a pylon_auth::org::OrgStore,
    /// Per-address pending SIWE nonces. Issued at
    /// `/api/auth/siwe/nonce`, consumed at `/api/auth/siwe/verify`.
    /// Gated on wasm32 — SIWE verification needs k256 (native-only).
    #[cfg(not(target_arch = "wasm32"))]
    pub siwe: &'a pylon_auth::siwe::NonceStore,
    /// Phone-number magic codes. Endpoints under `/api/auth/phone/...`.
    /// Gated on wasm32 — SMS providers use ureq.
    #[cfg(not(target_arch = "wasm32"))]
    pub phone_codes: &'a pylon_auth::phone::PhoneCodeStore,
    /// WebAuthn / passkey credentials + per-user challenge stash.
    /// Endpoints under `/api/auth/passkey/...`. Gated on wasm32 —
    /// WebAuthn assertion verification uses ring (native-only).
    #[cfg(not(target_arch = "wasm32"))]
    pub passkeys: &'a pylon_auth::webauthn::PasskeyStore,
    /// Single-use email-delivered tokens (password reset, email
    /// change, magic-link sign-in). Endpoints under
    /// `/api/auth/{password/reset,email/change,magic-link}/...`.
    pub verification: &'a pylon_auth::verification::VerificationStore,
    /// Append-only audit log for security-relevant events. Endpoints
    /// `/api/auth/audit` (current user) + `/api/auth/audit/tenant`
    /// (active tenant; admin-gated by your policy layer).
    pub audit: &'a pylon_auth::audit::AuditStore,
    /// Trusted-device records — the "remember this device for 30 days"
    /// store apps gate their TOTP step on. Endpoints under
    /// `/api/auth/trusted-devices` + the cookie is read by the
    /// auth middleware to populate `auth_ctx.is_trusted_device`.
    pub trusted_devices: &'a dyn pylon_auth::trusted_device::TrustedDeviceStore,
    /// Per-org SSO config + state. Endpoints under
    /// `/api/auth/orgs/<slug>/sso/...`. Gated on wasm32 — OIDC
    /// discovery fetches use ureq.
    #[cfg(not(target_arch = "wasm32"))]
    pub org_sso: &'a dyn pylon_auth::org_sso::OrgSsoStore,
    /// Per-org SAML 2.0 SSO config + state. Endpoints under
    /// `/api/auth/orgs/<slug>/saml/...`. Note: signature verification
    /// is OFF by default — see `pylon_auth::saml::signature_verification_bypassed`.
    /// Gated on wasm32 — samael needs openssl-sys (native-only).
    #[cfg(not(target_arch = "wasm32"))]
    pub saml: &'a dyn pylon_auth::saml::SamlStore,
    pub policy_engine: &'a PolicyEngine,
    pub change_log: &'a ChangeLog,
    pub notifier: &'a dyn ChangeNotifier,
    pub rooms: &'a dyn RoomOps,
    pub cache: &'a dyn CacheOps,
    pub pubsub: &'a dyn PubSubOps,
    pub jobs: &'a dyn JobOps,
    pub scheduler: &'a dyn SchedulerOps,
    pub workflows: &'a dyn WorkflowOps,
    pub files: &'a dyn FileOps,
    pub openapi: &'a dyn OpenApiGenerator,
    pub functions: Option<&'a dyn FnOps>,
    pub email: &'a dyn EmailSender,
    pub shards: Option<&'a dyn ShardOps>,
    pub plugin_hooks: &'a dyn PluginHookOps,
    pub auth_ctx: &'a AuthContext,
    /// Allowlist of origins (`scheme://host[:port]`) that the OAuth
    /// start endpoint will accept as `?callback=` / `?error_callback=`
    /// targets. Sourced from `PYLON_TRUSTED_ORIGINS` (comma-separated)
    /// at server boot. Borrowed from better-auth's `trustedOrigins`
    /// model — explicit allowlist, no implicit "same-origin trust" or
    /// env-var magic. Open redirects via OAuth are an easy bug to
    /// ship by accident; this list is the only thing standing between
    /// a misconfigured frontend and an attacker-controlled redirect.
    pub trusted_origins: &'a [String],
    pub is_dev: bool,
    /// Raw HTTP request headers (lowercased names). Used by the webhook
    /// action endpoint to pass the exact signing-relevant headers through
    /// to TypeScript actions. Empty slice on platforms that don't forward
    /// headers (e.g. internal calls).
    pub request_headers: &'a [(String, String)],
    /// Client IP as the runtime resolved it from the socket. Used as
    /// the rate-limit bucket key for unauthenticated callers — the
    /// alternative ("anon" string) puts every unauth request worldwide
    /// into one shared bucket, which lets one attacker starve every
    /// other anonymous caller. Empty string on platforms that don't
    /// expose a peer address.
    pub peer_ip: &'a str,
    /// Session cookie shape (name, domain, attrs). Handlers use this to
    /// emit Set-Cookie headers via [`RouterContext::add_response_header`]
    /// when they want a browser-bound session.
    pub cookie_config: &'a CookieConfig,
    /// Extra response headers handlers want to attach (e.g. Set-Cookie,
    /// Location). The runtime drains this after `route()` returns and
    /// merges them into the outgoing response. Interior mutability so
    /// handlers don't need a `&mut` ctx.
    pub response_headers: RefCell<Vec<(String, String)>>,
}

impl<'a> RouterContext<'a> {
    /// Queue a header to be added to the response built from this request.
    pub fn add_response_header(&self, name: impl Into<String>, value: impl Into<String>) {
        self.response_headers
            .borrow_mut()
            .push((name.into(), value.into()));
    }

    /// Drain the queued response headers. Runtime calls this once after
    /// `route()` returns, before constructing the wire response.
    pub fn take_response_headers(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.response_headers.borrow_mut())
    }

    /// Read the request's `Origin` header, if any. Browsers always send
    /// Origin on cross-origin XHR/fetch and on POSTs; non-browser
    /// callers (CLI, server-to-server) typically don't.
    pub fn request_origin(&self) -> Option<&str> {
        self.request_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("origin"))
            .map(|(_, v)| v.as_str())
    }

    /// Emit a session cookie when the request looks like it came from a
    /// browser (i.e. carries Origin). Non-browser callers still receive
    /// the JSON token in the body and ignore the missing cookie.
    /// Origin allowlisting is enforced at the runtime CSRF layer for
    /// state-changing methods, so handlers don't need to re-check here.
    ///
    /// When `cookie_config.domain` is set (e.g. `.pylonsync.com` for
    /// cross-subdomain SSO), we ALSO emit a Set-Cookie that expires
    /// any host-only cookie of the same name. This is the cleanup for
    /// a class of bug that bites whenever an operator flips
    /// `PYLON_COOKIE_DOMAIN` after launch: browsers hang onto the old
    /// narrower-scoped cookie and send BOTH on every request, with
    /// the server arbitrarily picking one per request — causing
    /// intermittent auth failures and policy denials. Without this
    /// guard, the only fix is "clear all your cookies and sign in
    /// again," which is unworkable for a real product.
    pub fn maybe_set_session_cookie(&self, token: &str) {
        if self.request_origin().is_some() {
            self.add_response_header("Set-Cookie", self.cookie_config.set_value(token));
            if let Some(clear) = self.cookie_config.host_only_clear_value() {
                self.add_response_header("Set-Cookie", clear);
            }
        }
    }

    /// Emit a session cookie unconditionally — for browser-302 flows
    /// (OAuth/SSO/SAML callbacks) where the request came from an IdP
    /// redirect and therefore won't carry Origin, so
    /// `maybe_set_session_cookie`'s Origin gate would skip it.
    ///
    /// Includes the host-only-clear pairing for the same reason
    /// `maybe_set_session_cookie` does — without it, an iPhone (or any
    /// device) that held a `${name}` cookie at the bare host BEFORE
    /// `PYLON_COOKIE_DOMAIN` got set keeps sending BOTH cookies forever
    /// and the server arbitrarily picks one per request. Manifested on
    /// iOS Chrome OAuth: the post-callback /dashboard load read the
    /// stale host-only token, found no session, bounced back to /login.
    pub fn set_browser_session_cookie(&self, token: &str) {
        self.add_response_header("Set-Cookie", self.cookie_config.set_value(token));
        if let Some(clear) = self.cookie_config.host_only_clear_value() {
            self.add_response_header("Set-Cookie", clear);
        }
    }
}

// ---------------------------------------------------------------------------
// OAuth callback shared logic (POST returns JSON, GET 302s with cookie)
// ---------------------------------------------------------------------------

pub(crate) struct OAuthError {
    pub(crate) status: u16,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// Shared OAuth code-for-session exchange. Returns the user_id + minted
/// Truncate (and elide) error strings before they end up in
/// `oauth_error_message` redirect URLs. Provider error bodies can be
/// huge or contain echoed-back request fields — keep the redirect
/// short and safe to log.
///
/// MAX is the budget for the *output*, including the ellipsis (3
/// bytes), so the slice itself caps at MAX-3.
fn truncate_for_redirect(s: &str) -> String {
    const MAX: usize = 240;
    if s.len() <= MAX {
        return s.to_string();
    }
    let budget = MAX - "…".len();
    let mut end = budget;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// session, or a structured error suitable for both JSON (POST) and
/// 302-redirect-with-error-param (GET) responses.
/// Complete an OAuth login. Caller must have ALREADY validated the
/// state token via `ctx.oauth_state.validate(...)` and is passing the
/// resulting `OAuthState` record in via... well, by virtue of having
/// called this function. State validation lives at the call site (not
/// here) because the GET /api/auth/callback/:provider handler needs
/// the validated record's callback URLs to know where to redirect on
/// both success and failure — and validate is single-use, so it can
/// only be called once per token.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn complete_oauth_login_pkce(
    ctx: &RouterContext,
    provider: &str,
    code: Option<&str>,
    pkce_verifier: Option<&str>,
    dev_email: Option<&str>,
    dev_name: Option<&str>,
) -> Result<(String, pylon_auth::Session), OAuthError> {
    let (userinfo, tokens) = if let Some(code) = code {
        let registry = pylon_auth::OAuthRegistry::shared();
        let config = registry.get(provider).cloned().ok_or_else(|| OAuthError {
            status: 404,
            code: "PROVIDER_NOT_FOUND",
            message: format!("OAuth provider \"{provider}\" not configured"),
        })?;
        let tokens = config
            .exchange_code_full_pkce(code, pkce_verifier)
            .map_err(|err| OAuthError {
                status: 502,
                code: "OAUTH_TOKEN_EXCHANGE_FAILED",
                // Sanitize: providers like to echo back the request body
                // on auth failures. The auth layer already redacts known
                // sensitive fields, but cap the length to keep stray
                // tokens out of redirect URLs.
                message: truncate_for_redirect(&format!("token exchange failed: {err}")),
            })?;
        // Apple identity lives in id_token, not a userinfo endpoint.
        // Pass both — the auth layer routes by spec.
        let info = config
            .fetch_userinfo_with_id_token(&tokens.access_token, tokens.id_token.as_deref())
            .map_err(|err| OAuthError {
                status: 502,
                code: "OAUTH_TOKEN_EXCHANGE_FAILED",
                message: truncate_for_redirect(&format!("userinfo fetch failed: {err}")),
            })?;
        (info, tokens)
    } else if ctx.is_dev {
        let email = dev_email.ok_or_else(|| OAuthError {
            status: 400,
            code: "MISSING_FIELD",
            message: "OAuth callback requires `code` (or `email` in dev mode)".into(),
        })?;
        // Dev path needs a stable provider_account_id so repeat
        // sign-ins land on the same Account row. Use the email itself
        // — predictable for tests, and a real provider would never
        // reuse an email as a sub.
        let info = pylon_auth::UserInfo {
            provider: provider.to_string(),
            provider_account_id: format!("dev:{email}"),
            email: email.to_string(),
            name: dev_name.map(String::from),
        };
        let tokens = pylon_auth::TokenSet {
            access_token: "dev_access_token".into(),
            refresh_token: None,
            id_token: None,
            expires_at: None,
            scope: None,
        };
        (info, tokens)
    } else {
        return Err(OAuthError {
            status: 400,
            code: "MISSING_FIELD",
            message: "OAuth callback requires an authorization `code` from the provider".into(),
        });
    };

    // Disposable email blocker. The provider vouched for the address,
    // but if it's still a known throwaway domain we don't want to
    // create the User row — the abuse signal is the same regardless
    // of which provider rubber-stamped it. PYLON_EMAIL_BLOCKLIST_DISABLED=1
    // turns this off.
    if pylon_auth::email_blocklist::is_disposable_email(&userinfo.email) {
        return Err(OAuthError {
            status: 400,
            code: "DISPOSABLE_EMAIL",
            message: "Sign in with a permanent email address. Disposable / throwaway providers aren't accepted.".into(),
        });
    }

    // Real-world bug this replaces: the previous formatter produced
    // strings like "1761811234Z" (epoch-seconds with a stray Z) that
    // SQLite happily stored as TEXT but PostgreSQL rejected as
    // invalid TIMESTAMPTZ — every Google sign-up against pylon-cloud
    // failed with USER_CREATE_FAILED. Use the kernel's ISO 8601
    // formatter for a value both backends parse cleanly.
    let now = chrono_now_iso();

    // Resolve user_id in priority order:
    //   1. Existing account link by (provider, provider_account_id) — the
    //      stable identity. Survives email changes on the provider side.
    //   2. Existing User row by email — account-linking-by-email. The
    //      classic "you signed up with email/password and now you're
    //      adding Google" flow.
    //   3. Create a new User.
    //
    // Crucially: every step that can fail (store.insert, store.update)
    // returns its error rather than silently using the email as user_id.
    // That swallow caused the "session for nonexistent user" bug — the
    // OAuth flow looked successful but the User row was never created
    // and /api/auth/me would resolve to a phantom identity.
    let user_id = if let Some(existing) = ctx
        .account_store
        .find_by_provider(provider, &userinfo.provider_account_id)
    {
        // Returning user via the same provider — refresh the token
        // bundle and reuse the linked user_id.
        let mut refreshed = pylon_auth::Account::new(existing.user_id.clone(), &userinfo, &tokens);
        refreshed.created_at = existing.created_at;
        ctx.account_store.upsert(&refreshed);
        existing.user_id
    } else if let Ok(Some(row)) = ctx.store.lookup(
        &ctx.store.manifest().auth.user.entity,
        "email",
        &userinfo.email,
    ) {
        // First-time link of this provider to an existing user (matched
        // by email). Stamp emailVerified opportunistically since the
        // provider just vouched for the address.
        let id = row["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            return Err(OAuthError {
                status: 500,
                code: "USER_LOOKUP_INVALID",
                message: "User row matched by email but had no id field".into(),
            });
        }
        if row.get("emailVerified").map_or(true, |v| v.is_null()) {
            // Best-effort — schemas without the field silently drop the
            // update. We do NOT bail on this error since the user
            // already existed and OAuth still succeeded.
            let _ = ctx.store.update(
                &ctx.store.manifest().auth.user.entity,
                &id,
                &serde_json::json!({ "emailVerified": now }),
            );
        }
        ctx.account_store
            .upsert(&pylon_auth::Account::new(id.clone(), &userinfo, &tokens));
        id
    } else {
        // Brand-new user. Create the User row + the Account link. Both
        // fail loudly — a silent failure here is what produced the
        // "session for nonexistent user" bug.
        let display_name = userinfo.name.as_deref().unwrap_or(&userinfo.email);
        let user_entity = ctx.store.manifest().auth.user.entity.clone();
        let id = ctx
            .store
            .insert(
                &user_entity,
                &serde_json::json!({
                    "email": userinfo.email,
                    "displayName": display_name,
                    "emailVerified": now,
                    "createdAt": now,
                }),
            )
            .map_err(|e| OAuthError {
                status: 500,
                code: "USER_CREATE_FAILED",
                // Preserve the full upstream code/message — a failed insert
                // is almost always "the User entity in your manifest has
                // a field this OAuth handler doesn't set" (NOT NULL
                // violation), and the operator needs to see exactly which
                // column.
                message: format!(
                    "failed to create User row for OAuth signup ({}): {}",
                    e.code, e.message
                ),
            })?;
        ctx.account_store
            .upsert(&pylon_auth::Account::new(id.clone(), &userinfo, &tokens));
        id
    };
    let session = ctx.session_store.create(user_id.clone());
    Ok((user_id, session))
}

/// Parse a `key=value&key=value` query string into a map. Uses
/// `query_decode` (NOT form_decode) — RFC 3986 says `+` is a literal
/// in URI query strings; only `application/x-www-form-urlencoded`
/// bodies decode `+` as space. OAuth state tokens that happen to
/// contain `+` (e.g. base64-with-padding) round-trip cleanly here.
pub(crate) fn parse_query(q: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(query_decode(k), query_decode(v));
    }
    out
}

/// Percent-decode a URI query-string segment. Treats `+` as a literal
/// `+` character (per RFC 3986 §3.4) — the `+` → space convention
/// only applies to `application/x-www-form-urlencoded` *bodies*, not
/// to URI query strings. Inlined `percent_decode` because the form-
/// body variant isn't used here.
fn query_decode(s: &str) -> String {
    percent_decode(s, false)
}

#[allow(dead_code)]
fn percent_decode(s: &str, plus_is_space: bool) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Redact an email for logging — keeps the first two characters of
/// the local-part + the domain, masks the rest. `alice@acme.com`
/// becomes `al***@acme.com`. Compliance-friendly (no full PII in
/// operator log aggregators) without losing all debuggability.
pub(crate) fn redact_email(email: &str) -> String {
    match email.find('@') {
        Some(at) => {
            let (user, domain) = email.split_at(at);
            let prefix_len = user.len().min(2);
            let prefix: String = user.chars().take(prefix_len).collect();
            format!("{prefix}***{domain}")
        }
        None => "***".to_string(),
    }
}

/// Build a redacted view of the manifest safe to serve to anonymous
/// callers. Drops the body of every policy expression — `allow_read`,
/// `allow_insert`, etc. — but keeps policy name + entity + action so
/// client tooling can map a "policy denied: ownerReadTodos" error to
/// the human label without seeing the raw rule.
fn public_manifest(m: &pylon_kernel::AppManifest) -> pylon_kernel::AppManifest {
    let mut out = m.clone();
    for p in out.policies.iter_mut() {
        p.allow = String::new();
        p.allow_read = None;
        p.allow_insert = None;
        p.allow_update = None;
        p.allow_delete = None;
    }
    out
}

pub(crate) fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// route() — the platform-agnostic request router
// ---------------------------------------------------------------------------

/// Route an HTTP request to the appropriate handler.
///
/// Returns `(status_code, response_body, content_type)`.
pub fn route(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    auth_token: Option<&str>,
) -> (u16, String, &'static str) {
    let (status, body) = route_inner(ctx, method, url, body, auth_token);
    (status, body, "application/json")
}

fn route_inner(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    auth_token: Option<&str>,
) -> (u16, String) {
    // CORS preflight
    if method == HttpMethod::Options {
        return (204, String::new());
    }

    // Email verification gate. When PYLON_REQUIRE_EMAIL_VERIFICATION=1,
    // an authed user with no emailVerified timestamp on their User row
    // can only hit /api/auth/* (so they can verify, log out, request a
    // resend) and the public manifest. Every other route returns 403
    // EMAIL_NOT_VERIFIED. Off by default — opt-in per deployment.
    //
    // Why here vs per-route: this is a session-state gate, not a
    // capability gate. Centralising it means a new route added later
    // doesn't accidentally bypass the check.
    if let Some((status, err)) = email_verification_gate(ctx, url) {
        return (status, err);
    }

    // GET /api/manifest
    // Public manifest. Clients need entity/field/route shapes to call
    // the API, but they do NOT need raw policy expressions — those are
    // server-enforcement details, and exposing them ("auth.userId ==
    // data.ownerId") tells an attacker exactly which condition to
    // satisfy. Strip allow_* expressions; keep policy NAMES so client
    // tooling can still surface "denied by ownerReadTodos" errors.
    // Admins get the full thing for tooling via ?full=1.
    if url.starts_with("/api/manifest") && method == HttpMethod::Get {
        let path = url.split('?').next().unwrap_or(url);
        if path == "/api/manifest" {
            let want_full = query_param(url, "full")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let manifest = ctx.store.manifest();
            let body = if want_full && ctx.auth_ctx.is_admin {
                serde_json::to_string(manifest).unwrap_or_else(|_| "{}".into())
            } else {
                serde_json::to_string(&public_manifest(manifest)).unwrap_or_else(|_| "{}".into())
            };
            return (200, body);
        }
    }

    // GET /api/openapi.json
    //
    // Gated to admin / dev mode. The full spec includes batch/transact
    // and the entity-API surface, which is useful reconnaissance for
    // an attacker probing app shape. Public clients don't need it (they
    // use the typed SDK), so admins-only is the right default. Dev
    // mode keeps it open so hand-curl explorations still work.
    // Caught in the 2026-05-10 codex pass-3 audit (P3 NEW).
    if url == "/api/openapi.json" && method == HttpMethod::Get {
        if !ctx.is_dev && !ctx.auth_ctx.is_admin {
            return (
                404,
                json_error("NOT_FOUND", "OpenAPI spec is admin-only outside dev mode"),
            );
        }
        return (200, ctx.openapi.generate(""));
    }

    // -----------------------------------------------------------------------
    // Auth routes — handled by crates/router/src/routes/auth.rs.
    // /api/auth/* (sessions, OAuth, magic-link, password, email verify,
    // /me, /providers, /sessions, refresh).
    // -----------------------------------------------------------------------
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(r) = routes::auth::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    // On wasm32 the auth.rs module is excluded because it depends
    // on native-only crypto (ring/k256/argon2/samael/ureq). The
    // pure-data routes (/me, /session GET/DELETE, /sessions,
    // /refresh) could be served from the wasm32 build but
    // currently aren't — they share auth.rs with the network paths
    // and carving them out cleanly is a separate refactor. Until
    // then, return an explicit typed 501 so a Workers customer
    // hitting /api/auth/* sees "feature unavailable on this
    // target" instead of a silent NOT_FOUND fallthrough that
    // looks like a routing bug.
    #[cfg(target_arch = "wasm32")]
    if url.starts_with("/api/auth/") {
        return (
            501,
            json_error(
                "AUTH_UNAVAILABLE_ON_WORKERS",
                "Auth flows (OAuth/SAML/magic-link/email-verify/passkey/phone) require native crypto deps (ring/samael/argon2/etc.) that don't cross-compile to wasm32. Deploy on Fly for full auth surface, or use bearer-token auth on Workers via custom routes.",
            ),
        );
    }

    // -----------------------------------------------------------------------
    // Sync + GDPR — handled by crates/router/src/routes/sync.rs.
    // /api/sync/{pull,push}, /api/admin/users/:id/{export,purge}.
    // -----------------------------------------------------------------------
    if let Some(r) = routes::sync::handle(ctx, method, url, body, auth_token) {
        return r;
    }

    // -----------------------------------------------------------------------
    // Rooms — handled by crates/router/src/routes/rooms.rs.
    // /api/rooms/{join,leave,presence,broadcast}, /api/rooms[/<room>].
    // -----------------------------------------------------------------------
    if let Some(r) = routes::rooms::handle(ctx, method, url, body, auth_token) {
        return r;
    }

    // -----------------------------------------------------------------------
    // Link / Files / CRDT — handled by routes/{links,files,crdt}.rs.
    // /api/{link,unlink}, /api/files/{upload,<id>}, /api/crdt/<entity>/<row>.
    // -----------------------------------------------------------------------
    if let Some(r) = routes::links::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    if let Some(r) = routes::files::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    if let Some(r) = routes::crdt::handle(ctx, method, url, body, auth_token) {
        return r;
    }

    // -----------------------------------------------------------------------
    // Queries / Actions / Admin data / Search — handled by:
    //   routes/queries.rs   (transact, query/:e, lookup, aggregate, query)
    //   routes/actions.rs   (/api/actions/<name>)
    //   routes/admin_data.rs (export, import)
    //   routes/search.rs    (/api/search/<entity>)
    // -----------------------------------------------------------------------
    if let Some(r) = routes::queries::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    if let Some(r) = routes::actions::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    if let Some(r) = routes::admin_data::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(r) = routes::auth_admin::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    if let Some(r) = routes::ops_admin::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    if let Some(r) = routes::search::handle(ctx, method, url, body, auth_token) {
        return r;
    }

    // -----------------------------------------------------------------------
    // Entity CRUD + cursor + batch — handled by routes/entities.rs.
    // /api/entities/<entity>[/<id>], /api/entities/<entity>/cursor,
    // /api/batch.
    // -----------------------------------------------------------------------
    if let Some(r) = routes::entities::handle(ctx, method, url, body, auth_token) {
        return r;
    }

    // -----------------------------------------------------------------------
    // Infra / Functions / Shards / Workflows / AI — handled by:
    //   routes/infra.rs       (cache, pubsub, jobs, scheduler)
    //   routes/functions.rs   (/api/fn, /api/fn/traces, /api/webhooks/<>)
    //   routes/shards.rs      (/api/shards*)
    //   routes/workflows.rs   (/api/workflows*)
    //   routes/ai.rs          (/api/ai/complete shim — runtime owns stream)
    // -----------------------------------------------------------------------
    if let Some(r) = routes::infra::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    if let Some(r) = routes::functions::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    if let Some(r) = routes::shards::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    if let Some(r) = routes::workflows::handle(ctx, method, url, body, auth_token) {
        return r;
    }
    if let Some(r) = routes::ai::handle(ctx, method, url, body, auth_token) {
        return r;
    }

    // -----------------------------------------------------------------------
    // Fallback
    // -----------------------------------------------------------------------

    (
        404,
        json_error_with_hint(
            "NOT_FOUND",
            &format!("No API route matches {url}"),
            "Available endpoints: /api/entities/<entity>, /api/actions/<name>, /api/query, /api/auth/*, /api/sync/*, /api/files/*, /api/cache, /api/pubsub/*, /api/jobs, /api/scheduler, /api/workflows, /api/ai/*, /studio",
        ),
    )
}

// ---------------------------------------------------------------------------
// Entity CRUD helpers
// ---------------------------------------------------------------------------

pub(crate) fn handle_list(store: &dyn DataStore, entity: &str, url: &str) -> (u16, String) {
    let qp = parse_query_params(url);

    // ---- Pagination ---------------------------------------------------
    // Two parameter shapes are accepted:
    //   ?limit=&offset=         legacy
    //   ?page=&per_page=        Studio + Refine convention
    // When `page` is supplied the response is an envelope with
    // `data` + `total` + `page` + `per_page`; otherwise we return the
    // legacy `{data, count, offset, limit}` shape so existing clients
    // keep working unchanged.
    let limit: Option<usize> = qp.get("limit").and_then(|v| v.parse().ok());
    let offset: usize = qp.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    let page: Option<usize> = qp.get("page").and_then(|v| v.parse().ok());
    let per_page: Option<usize> = qp.get("per_page").and_then(|v| v.parse().ok());
    let paginated = page.is_some() || per_page.is_some();

    let (effective_limit, effective_offset) = if paginated {
        let pp = per_page.unwrap_or(25).max(1).min(1000);
        let p = page.unwrap_or(1).max(1);
        (Some(pp), (p - 1) * pp)
    } else {
        (limit, offset)
    };

    // ---- Build base filter (per-field) -------------------------------
    let mut filter = serde_json::Map::new();
    for (k, v) in &qp {
        // filter[<field>]=<value>
        if let Some(field) = k.strip_prefix("filter[").and_then(|s| s.strip_suffix(']')) {
            // Try numeric / bool coercion so `filter[active]=true` matches
            // a stored boolean. Fall back to string equality otherwise.
            let parsed = parse_query_value(v);
            filter.insert(field.to_string(), parsed);
        }
    }

    // ---- Sort --------------------------------------------------------
    if let Some(sort_field) = qp.get("sort") {
        let order = qp
            .get("order")
            .map(|s| s.as_str())
            .unwrap_or("asc")
            .to_lowercase();
        let dir = if order == "desc" { "desc" } else { "asc" };
        let mut order_obj = serde_json::Map::new();
        order_obj.insert(sort_field.clone(), serde_json::Value::String(dir.into()));
        filter.insert("$order".into(), serde_json::Value::Object(order_obj));
    }

    // ---- Text search ------------------------------------------------
    // `q` maps to the storage-layer $search clause, which uses FTS5 on
    // SQLite when the entity declares `search.text`. Entities without a
    // search config will get an error from the storage layer; we
    // surface that as a soft fallback (drop $search and continue) so a
    // search input on an unsearchable entity simply ignores the query
    // instead of returning a hard 500.
    if let Some(q) = qp.get("q") {
        if !q.trim().is_empty() {
            filter.insert("$search".into(), serde_json::Value::String(q.clone()));
        }
    }

    // ---- Total count (paginated mode only) ---------------------------
    let total = if paginated {
        let mut count_filter = filter.clone();
        // Strip pagination + ordering for the count query.
        count_filter.remove("$limit");
        count_filter.remove("$offset");
        count_filter.remove("$order");
        match store.query_filtered(entity, &serde_json::Value::Object(count_filter)) {
            Ok(rows) => Some(rows.len()),
            Err(_) => None,
        }
    } else {
        None
    };

    // ---- Apply pagination to the data filter -------------------------
    if let Some(l) = effective_limit {
        filter.insert("$limit".into(), serde_json::json!(l));
    }
    if effective_offset > 0 {
        filter.insert("$offset".into(), serde_json::json!(effective_offset));
    }
    let filter = serde_json::Value::Object(filter);

    // ---- Execute -----------------------------------------------------
    let rows_result = store.query_filtered(entity, &filter);
    let rows = match rows_result {
        Ok(r) => r,
        Err(e) => {
            // FTS-not-configured failures: retry without $search so the
            // unsearchable entity still loads on a mistargeted query.
            if e.code == "SEARCH_NOT_CONFIGURED" || e.code == "FTS_UNSUPPORTED" {
                let mut retry = filter.clone();
                if let Some(obj) = retry.as_object_mut() {
                    obj.remove("$search");
                }
                match store.query_filtered(entity, &retry) {
                    Ok(r) => r,
                    Err(e) => return (400, json_error(&e.code, &e.message)),
                }
            } else {
                return (400, json_error(&e.code, &e.message));
            }
        }
    };

    let auth_user = &store.manifest().auth.user;
    let rows: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| maybe_project_user_row(entity, r, auth_user))
        .collect();

    if paginated {
        let p = page.unwrap_or(1);
        let pp = per_page.unwrap_or(25);
        (
            200,
            serde_json::json!({
                "data": rows,
                "total": total,
                "page": p,
                "per_page": pp,
            })
            .to_string(),
        )
    } else {
        let count = rows.len();
        (
            200,
            serde_json::json!({
                "data": rows,
                "count": count,
                "offset": offset,
                "limit": limit,
            })
            .to_string(),
        )
    }
}

/// Parse the `?key=value&key=value` portion of a URL into a flat map.
/// Values are URL-decoded (best-effort — unknown escapes pass through).
/// Repeated keys keep only the *last* value, which matches every
/// existing call site's expectation.
fn parse_query_params(url: &str) -> std::collections::HashMap<String, String> {
    let q = url.split('?').nth(1).unwrap_or("");
    let mut out = std::collections::HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut it = pair.splitn(2, '=');
        let k = it.next().unwrap_or("");
        let v = it.next().unwrap_or("");
        let dk = url_decode(k);
        let dv = url_decode(v);
        out.insert(dk, dv);
    }
    out
}

fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'+' {
            out.push(' ');
            i += 1;
            continue;
        }
        if b == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h * 16 + l) as u8) as char);
                i += 3;
                continue;
            }
        }
        out.push(b as char);
        i += 1;
    }
    out
}

/// Best-effort coercion of a query-string value to a JSON Value.
/// `"true" / "false"` → bool; pure-digit strings → integer; otherwise
/// kept as a string. This lets `filter[active]=true` match a stored
/// boolean without forcing the operator to think about types.
fn parse_query_value(s: &str) -> serde_json::Value {
    if s == "true" {
        return serde_json::Value::Bool(true);
    }
    if s == "false" {
        return serde_json::Value::Bool(false);
    }
    if let Ok(n) = s.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    if let Ok(n) = s.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(n) {
            return serde_json::Value::Number(num);
        }
    }
    serde_json::Value::String(s.to_string())
}

pub(crate) fn handle_get(store: &dyn DataStore, entity: &str, id: &str) -> (u16, String) {
    match store.get_by_id(entity, id) {
        Ok(Some(row)) => {
            let projected = maybe_project_user_row(entity, row, &store.manifest().auth.user);
            (
                200,
                serde_json::to_string(&projected).unwrap_or_else(|_| "{}".into()),
            )
        }
        Ok(None) => (
            404,
            json_error("NOT_FOUND", &format!("{entity} with id \"{id}\" not found")),
        ),
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

pub(crate) fn handle_insert(ctx: &RouterContext, entity: &str, body: &str) -> (u16, String) {
    let mut data: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json_error_safe(
                    "INVALID_JSON",
                    "Invalid request body",
                    &format!("Invalid JSON: {e}"),
                ),
            )
        }
    };
    // Run plugin `before_insert` hooks. Registered plugins (validation,
    // timestamps, slugify) mutate `data` here; a rejected hook aborts
    // the write with their status + error payload.
    if let Err((status, code, msg)) =
        ctx.plugin_hooks
            .before_insert(entity, &mut data, ctx.auth_ctx)
    {
        return (status, json_error(&code, &msg));
    }
    match ctx.store.insert(entity, &data) {
        Ok(id) => {
            // Re-read so the broadcast event carries the full
            // materialized row (server-stamped defaults, plugin-added
            // fields like tenantId, SQL DEFAULTs). Partial broadcasts
            // trip the per-client policy filter on the WS shard
            // worker — `auth.tenantId == row.tenantId` evaluates
            // false against a row that's missing tenantId. See the
            // same fix on TxStore::insert.
            //
            // `Ok(None)` = concurrent delete (or a trigger removed
            // the row mid-write). Skip the broadcast; the delete
            // will fire its own event and emitting an Insert here
            // would resurrect a row the server already considers
            // gone.
            match ctx.store.get_by_id(entity, &id) {
                Ok(Some(full)) => {
                    let seq =
                        ctx.change_log
                            .append(entity, &id, ChangeKind::Insert, Some(full.clone()));
                    broadcast_change_with_crdt(
                        ctx.notifier,
                        ctx.store,
                        seq,
                        entity,
                        &id,
                        ChangeKind::Insert,
                        Some(&full),
                    );
                }
                Ok(None) => {
                    tracing::warn!(
                        "[handle_insert] post-write re-read returned None for {entity}/{id} — concurrent delete; skipping broadcast"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "[handle_insert] post-write re-read failed for {entity}/{id} ({}): broadcasting partial payload",
                        e.message
                    );
                    let seq =
                        ctx.change_log
                            .append(entity, &id, ChangeKind::Insert, Some(data.clone()));
                    broadcast_change_with_crdt(
                        ctx.notifier,
                        ctx.store,
                        seq,
                        entity,
                        &id,
                        ChangeKind::Insert,
                        Some(&data),
                    );
                }
            }
            ctx.plugin_hooks
                .after_insert(entity, &id, &data, ctx.auth_ctx);
            (201, serde_json::json!({"id": id}).to_string())
        }
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

pub(crate) fn handle_update(
    ctx: &RouterContext,
    entity: &str,
    id: &str,
    body: &str,
) -> (u16, String) {
    let mut data: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json_error_safe(
                    "INVALID_JSON",
                    "Invalid request body",
                    &format!("Invalid JSON: {e}"),
                ),
            )
        }
    };
    if let Err((status, code, msg)) =
        ctx.plugin_hooks
            .before_update(entity, id, &mut data, ctx.auth_ctx)
    {
        return (status, json_error(&code, &msg));
    }
    match ctx.store.update(entity, id, &data) {
        Ok(true) => {
            // Re-read post-update so the broadcast carries the full
            // current row state, not just the partial patch. See the
            // commentary on handle_insert above — `Ok(None)` = the
            // row was concurrently deleted; skip the broadcast to
            // avoid resurrecting it in client replicas.
            match ctx.store.get_by_id(entity, id) {
                Ok(Some(full)) => {
                    let seq =
                        ctx.change_log
                            .append(entity, id, ChangeKind::Update, Some(full.clone()));
                    broadcast_change_with_crdt(
                        ctx.notifier,
                        ctx.store,
                        seq,
                        entity,
                        id,
                        ChangeKind::Update,
                        Some(&full),
                    );
                }
                Ok(None) => {
                    tracing::warn!(
                        "[handle_update] post-write re-read returned None for {entity}/{id} — concurrent delete; skipping broadcast"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "[handle_update] post-write re-read failed for {entity}/{id} ({}): broadcasting partial payload",
                        e.message
                    );
                    let seq =
                        ctx.change_log
                            .append(entity, id, ChangeKind::Update, Some(data.clone()));
                    broadcast_change_with_crdt(
                        ctx.notifier,
                        ctx.store,
                        seq,
                        entity,
                        id,
                        ChangeKind::Update,
                        Some(&data),
                    );
                }
            }
            ctx.plugin_hooks
                .after_update(entity, id, &data, ctx.auth_ctx);
            (200, serde_json::json!({"updated": true}).to_string())
        }
        Ok(false) => (
            404,
            json_error("NOT_FOUND", &format!("{entity}/{id} not found")),
        ),
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

pub(crate) fn handle_delete(ctx: &RouterContext, entity: &str, id: &str) -> (u16, String) {
    if let Err((status, code, msg)) = ctx.plugin_hooks.before_delete(entity, id, ctx.auth_ctx) {
        return (status, json_error(&code, &msg));
    }
    match ctx.store.delete(entity, id) {
        Ok(true) => {
            let seq = ctx.change_log.append(entity, id, ChangeKind::Delete, None);
            broadcast_change(ctx.notifier, seq, entity, id, ChangeKind::Delete, None);
            ctx.plugin_hooks.after_delete(entity, id, ctx.auth_ctx);
            (200, serde_json::json!({"deleted": true}).to_string())
        }
        Ok(false) => (
            404,
            json_error("NOT_FOUND", &format!("{entity}/{id} not found")),
        ),
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn broadcast_change(
    notifier: &dyn ChangeNotifier,
    seq: u64,
    entity: &str,
    row_id: &str,
    kind: ChangeKind,
    data: Option<&serde_json::Value>,
) {
    let event = pylon_sync::ChangeEvent {
        seq,
        entity: entity.to_string(),
        row_id: row_id.to_string(),
        kind,
        data: data.cloned(),
        timestamp: String::new(),
    };
    notifier.notify(&event);
}

/// Convenience: emit BOTH the JSON change event AND the binary CRDT
/// snapshot frame after a successful insert/update on a CRDT-mode
/// entity. The CRDT snapshot is fetched via [`DataStore::crdt_snapshot`];
/// for `crdt: false` entities (the LWW opt-out) it returns `Ok(None)`
/// and we skip the binary broadcast cleanly.
///
/// Delete operations don't ship a CRDT frame — Loro doesn't have a
/// "row gone" concept; the JSON change event with `kind: "delete"` is
/// the canonical signal. Clients drop the LoroDoc on receipt.
pub fn broadcast_change_with_crdt(
    notifier: &dyn ChangeNotifier,
    store: &dyn DataStore,
    seq: u64,
    entity: &str,
    row_id: &str,
    kind: ChangeKind,
    data: Option<&serde_json::Value>,
) {
    broadcast_change(notifier, seq, entity, row_id, kind.clone(), data);
    if matches!(kind, ChangeKind::Delete) {
        return;
    }
    // Per-row "last VV broadcast" map. Pre-fix every CRDT write
    // sent the full snapshot (~200-500 bytes after compaction).
    // After: first broadcast for a row still ships the snapshot
    // (so new + existing subscribers all agree on a baseline),
    // but every subsequent write ships an incremental delta from
    // the last broadcast's VV — typically 50-150 bytes for a
    // single field change. Loro's import is idempotent so a new
    // subscriber that just received the snapshot can also apply
    // the next delta (the ops already in their snapshot are no-
    // ops on import).
    //
    // The map is process-wide because the notifier surface
    // doesn't carry per-row state, but the data is small (key =
    // ~40 bytes for entity+row id, value = ~10s of bytes for a
    // postcard-encoded VV) so a long-running pylon with tens of
    // thousands of CRDT rows holds < 1MB here.
    use std::sync::Mutex;
    static LAST_VV: std::sync::OnceLock<
        Mutex<std::collections::HashMap<(String, String), Vec<u8>>>,
    > = std::sync::OnceLock::new();
    let table = LAST_VV.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let key = (entity.to_string(), row_id.to_string());
    let prev_vv: Option<Vec<u8>> = table.lock().ok().and_then(|m| m.get(&key).cloned());

    let (frame_payload, fell_back_to_snapshot) = match prev_vv {
        Some(since) => match store.crdt_update_since(entity, row_id, &since) {
            Ok(Some(delta)) => (Some(delta), false),
            // Backend doesn't support deltas (PG today) OR no
            // bytes returned — fall back to snapshot.
            _ => match store.crdt_snapshot(entity, row_id) {
                Ok(Some(snap)) => (Some(snap), true),
                _ => (None, true),
            },
        },
        None => match store.crdt_snapshot(entity, row_id) {
            Ok(Some(snap)) => (Some(snap), true),
            _ => (None, true),
        },
    };

    if let Some(payload) = frame_payload {
        // notify_crdt takes (entity, row_id, snapshot|delta) — the
        // wire framing is owned by the notifier, but it always
        // ships frame type 0x10 (snapshot) today. Pylon's
        // client-side Loro import treats 0x10 and 0x11 the same
        // (both go through `doc.import(bytes)` which is shape-
        // tolerant), so the wire-frame type byte is informational
        // for now — a future binary protocol revision can switch
        // on it. We pass the bytes through unchanged; the
        // delta-vs-snapshot decision was made here.
        let _ = fell_back_to_snapshot; // intentionally unused — see note above
        notifier.notify_crdt(entity, row_id, &payload);
        // Stash the post-write VV so the NEXT write's delta is
        // computed against this point. Skip the update on
        // backends that don't support VV tracking (returns None).
        if let Ok(Some(new_vv)) = store.crdt_vv(entity, row_id) {
            if let Ok(mut m) = table.lock() {
                m.insert(key, new_vv);
            }
        }
    }
}

/// Tiny lowercase-hex decoder. Returns `None` on any malformed input
/// (odd length, non-hex character). Used by the CRDT push endpoint to
/// turn the JSON `{update: "<hex>"}` payload back into binary Loro
/// bytes without pulling in a base64 dep just for one route.
pub(crate) fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn json_error(code: &str, message: &str) -> String {
    serde_json::json!({"error": {"code": code, "message": message}}).to_string()
}

/// Conventional field names that link a row to a user. Used by the GDPR
/// export/purge endpoints to discover referencing rows without requiring
/// app authors to annotate their schema. Apps that store user references
/// under non-conventional names must supply their own purge hook — the
/// default won't find them, and GDPR-compliant deletes require a full
/// sweep. We log a warning on export when entities appear to reference
/// users through custom names not in this list.
const USER_REF_FIELDS: &[&str] = &[
    "userId",
    "user_id",
    "authorId",
    "author_id",
    "ownerId",
    "owner_id",
    "createdBy",
    "created_by",
];

/// Build a data-subject export for `user_id`. Returns a JSON envelope with
/// every row referencing the user across every manifest entity, plus the
/// User row itself when present. Format is stable — clients can diff
/// exports across time to see what's changed.
pub(crate) fn gdpr_export(ctx: &RouterContext, user_id: &str) -> (u16, String) {
    let manifest = ctx.store.manifest();
    let mut entities = serde_json::Map::new();

    // The User row itself (if the schema has a User entity).
    if let Ok(Some(user_row)) = ctx.store.get_by_id("User", user_id) {
        entities.insert("User".to_string(), serde_json::json!([user_row]));
    }

    // Every entity that has a user-ref field: list all rows matching.
    for ent in &manifest.entities {
        if ent.name == "User" {
            continue; // Already captured above.
        }
        let user_field = ent
            .fields
            .iter()
            .find(|f| USER_REF_FIELDS.contains(&f.name.as_str()));
        let Some(field) = user_field else { continue };
        let filter = serde_json::json!({ &field.name: user_id });
        match ctx.store.query_filtered(&ent.name, &filter) {
            Ok(rows) if !rows.is_empty() => {
                entities.insert(ent.name.clone(), serde_json::Value::Array(rows));
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("[gdpr] export: query {} failed: {}", ent.name, e.message);
            }
        }
    }

    let envelope = serde_json::json!({
        "user_id": user_id,
        "exported_at": pylon_kernel::util::now_iso(),
        "entities": entities,
    });
    (200, envelope.to_string())
}

/// Hard-delete every row tied to `user_id` and revoke all active sessions.
/// Best-effort: a per-entity error is logged and counted but does not abort
/// the sweep — partial success is better than leaving half the footprint.
pub(crate) fn gdpr_purge(ctx: &RouterContext, user_id: &str) -> (u16, String) {
    let manifest = ctx.store.manifest();
    let mut deleted: u64 = 0;
    let mut errors: Vec<String> = Vec::new();

    // Delete the User row itself first.
    if let Ok(true) = ctx.store.delete("User", user_id) {
        deleted += 1;
        // Synthetic change event so sync clients notice.
        let seq = ctx
            .change_log
            .append("User", user_id, ChangeKind::Delete, None);
        broadcast_change(ctx.notifier, seq, "User", user_id, ChangeKind::Delete, None);
    }

    // Referencing rows.
    for ent in &manifest.entities {
        if ent.name == "User" {
            continue;
        }
        let Some(field) = ent
            .fields
            .iter()
            .find(|f| USER_REF_FIELDS.contains(&f.name.as_str()))
        else {
            continue;
        };
        let filter = serde_json::json!({ &field.name: user_id });
        let rows = match ctx.store.query_filtered(&ent.name, &filter) {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("query {}: {}", ent.name, e.message));
                continue;
            }
        };
        for row in rows {
            let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            match ctx.store.delete(&ent.name, id) {
                Ok(true) => {
                    deleted += 1;
                    let seq = ctx
                        .change_log
                        .append(&ent.name, id, ChangeKind::Delete, None);
                    broadcast_change(ctx.notifier, seq, &ent.name, id, ChangeKind::Delete, None);
                }
                Ok(false) => {}
                Err(e) => errors.push(format!("delete {}/{}: {}", ent.name, id, e.message)),
            }
        }
    }

    // Invalidate every active session for the user. Even after the row is
    // gone, an in-flight session token would let a purged user keep acting.
    let revoked = ctx.session_store.revoke_all_for_user(user_id);

    let resp = serde_json::json!({
        "user_id": user_id,
        "rows_deleted": deleted,
        "sessions_revoked": revoked,
        "errors": errors,
        "purged_at": pylon_kernel::util::now_iso(),
    });
    (200, resp.to_string())
}

/// Project a User-entity row down to the safe-for-client field set,
/// honoring the manifest's `auth.user.expose`/`hide` config. Defaults
/// strip `passwordHash` + anything starting with `_`.
///
/// Use this on every read path that can return a User row (entity GET,
/// entity LIST, sync change events, etc.). Without it, a permissive
/// User read policy — required for cross-user displayName lookups in
/// chat-style apps — would also leak `passwordHash`.
///
/// Non-User entities pass through unchanged; the projection only
/// fires when `entity == auth_user.entity`.
pub fn maybe_project_user_row(
    entity: &str,
    row: serde_json::Value,
    auth_user: &pylon_kernel::ManifestAuthUserConfig,
) -> serde_json::Value {
    if entity != auth_user.entity {
        return row;
    }
    project_user_fields(row, auth_user)
}

/// Same projection logic as `crate::routes::auth::project_user_row`,
/// but lifted here so non-auth routes can use it. Callers must have
/// already confirmed the row belongs to the User entity.
fn project_user_fields(
    row: serde_json::Value,
    cfg: &pylon_kernel::ManifestAuthUserConfig,
) -> serde_json::Value {
    let serde_json::Value::Object(obj) = row else {
        return row;
    };
    let filtered: serde_json::Map<String, serde_json::Value> = obj
        .into_iter()
        .filter(|(k, _)| {
            if k == "id" {
                return true;
            }
            if !cfg.expose.is_empty() && !cfg.expose.iter().any(|f| f == k) {
                return false;
            }
            if k == "passwordHash" || k.starts_with('_') {
                return false;
            }
            if cfg.hide.iter().any(|f| f == k) {
                return false;
            }
            true
        })
        .collect();
    serde_json::Value::Object(filtered)
}

/// Gate a route behind admin auth. Returns `Some(error_response)` if the
/// caller is NOT admin, `None` if they are. Use at the top of any control-
/// plane handler (jobs, workflows, sync push, etc) that shouldn't be open
/// to arbitrary clients.
pub(crate) fn require_admin(ctx: &RouterContext) -> Option<(u16, String)> {
    if ctx.auth_ctx.is_admin {
        None
    } else {
        Some((
            403,
            json_error(
                "FORBIDDEN",
                "this endpoint requires admin auth (PYLON_ADMIN_TOKEN)",
            ),
        ))
    }
}

/// Email verification gate. Triggers only when:
///   - PYLON_REQUIRE_EMAIL_VERIFICATION=1 is set on the runtime
///   - The caller has a user session (admins + anonymous bypass)
///   - The User row's `emailVerified` field is null
///   - The request is not to /api/auth/* (so the user can verify)
///   - The request is not /api/manifest (clients need shapes to render
///     the verify-email page itself)
///
/// On miss, returns 403 EMAIL_NOT_VERIFIED with a hint pointing at
/// /api/auth/email/send-verification. Adds one extra DB read per
/// gated request — the user lookup. Acceptable: this is opt-in.
fn email_verification_gate(ctx: &RouterContext, url: &str) -> Option<(u16, String)> {
    if std::env::var("PYLON_REQUIRE_EMAIL_VERIFICATION").as_deref() != Ok("1") {
        return None;
    }
    // Anonymous + admin contexts: nothing to verify.
    let user_id = ctx.auth_ctx.user_id.as_deref()?;
    if ctx.auth_ctx.is_admin {
        return None;
    }
    // Auth surfaces stay open so the user can verify, log out, manage
    // sessions, request a new code. /api/manifest is needed for the
    // dashboard chrome to even render the verify-email page.
    let path = url.split('?').next().unwrap_or(url);
    if path.starts_with("/api/auth/") || path == "/api/manifest" {
        return None;
    }
    // Resolve emailVerified for this user. If we can't find the row
    // (race with deletion, etc.), fail closed — the request would
    // bounce on the underlying entity policy anyway.
    let entity = &ctx.store.manifest().auth.user.entity;
    let verified = match ctx.store.get_by_id(entity, user_id) {
        Ok(Some(row)) => !row
            .get("emailVerified")
            .map(|v| v.is_null())
            .unwrap_or(true),
        _ => false,
    };
    if verified {
        return None;
    }
    Some((
        403,
        json_error_with_hint(
            "EMAIL_NOT_VERIFIED",
            "Verify your email address before continuing.",
            "POST /api/auth/email/send-verification → POST /api/auth/email/verify",
        ),
    ))
}

/// Gate a route behind "any authenticated identity". Returns `Some(err)` when
/// the caller has neither a user session nor an admin token. Used for the
/// rooms API, which previously let unauthenticated clients enumerate rooms
/// and read membership rosters — a silent presence-data leak.
pub(crate) fn require_auth(ctx: &RouterContext) -> Option<(u16, String)> {
    if ctx.auth_ctx.is_admin || ctx.auth_ctx.user_id.is_some() {
        None
    } else {
        Some((
            401,
            json_error("AUTH_REQUIRED", "authenticated session required"),
        ))
    }
}

pub fn json_error_with_hint(code: &str, message: &str, hint: &str) -> String {
    serde_json::json!({"error": {"code": code, "message": message, "hint": hint}}).to_string()
}

pub fn json_error_safe(code: &str, user_message: &str, internal: &str) -> String {
    tracing::warn!("[error] {code}: {internal}");
    json_error(code, user_message)
}

/// Parse a JSON request body, returning a 400 error tuple on failure.
pub(crate) fn parse_json(body: &str) -> Result<serde_json::Value, (u16, String)> {
    serde_json::from_str(body).map_err(|e| {
        (
            400,
            json_error_safe(
                "INVALID_JSON",
                "Invalid request body",
                &format!("Invalid JSON: {e}"),
            ),
        )
    })
}

/// Extract a query parameter value from a URL string.
pub(crate) fn query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let search = format!("{key}=");
    url.split(&search).nth(1).and_then(|s| s.split('&').next())
}

pub(crate) fn chrono_now_iso() -> String {
    pylon_kernel::util::now_iso()
}

// ---------------------------------------------------------------------------
// Integration tests — auth-bypass regressions
// ---------------------------------------------------------------------------
//
// These tests lock in the auth-gate fixes from the security review so future
// changes can't silently re-introduce them. Each test builds a minimal
// RouterContext with stub implementations of the service traits and exercises
// a previously-vulnerable route.

#[cfg(test)]
mod auth_gate_tests {
    use super::*;
    use pylon_auth::{AuthContext, CookieConfig, MagicCodeStore, OAuthStateStore, SessionStore};
    use pylon_kernel::{AppManifest, MANIFEST_VERSION};
    use pylon_policy::PolicyEngine;
    use pylon_sync::ChangeLog;

    // -----------------------------------------------------------------------
    // Minimal stubs for every service trait so we can build a RouterContext
    // without wiring up a real runtime.
    // -----------------------------------------------------------------------

    struct StubDataStore {
        manifest: AppManifest,
    }
    impl pylon_http::DataStore for StubDataStore {
        fn manifest(&self) -> &AppManifest {
            &self.manifest
        }
        fn insert(
            &self,
            _entity: &str,
            _data: &serde_json::Value,
        ) -> Result<String, pylon_http::DataError> {
            Ok("stub-id".to_string())
        }
        fn get_by_id(
            &self,
            _entity: &str,
            _id: &str,
        ) -> Result<Option<serde_json::Value>, pylon_http::DataError> {
            Ok(None)
        }
        fn list(&self, _entity: &str) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
            Ok(Vec::new())
        }
        fn list_after(
            &self,
            _entity: &str,
            _after: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
            Ok(Vec::new())
        }
        fn update(
            &self,
            _entity: &str,
            _id: &str,
            _data: &serde_json::Value,
        ) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn delete(&self, _entity: &str, _id: &str) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn lookup(
            &self,
            _entity: &str,
            _field: &str,
            _value: &str,
        ) -> Result<Option<serde_json::Value>, pylon_http::DataError> {
            Ok(None)
        }
        fn link(
            &self,
            _entity: &str,
            _id: &str,
            _relation: &str,
            _target_id: &str,
        ) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn unlink(
            &self,
            _entity: &str,
            _id: &str,
            _relation: &str,
        ) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn query_filtered(
            &self,
            _entity: &str,
            _filter: &serde_json::Value,
        ) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
            Ok(Vec::new())
        }
        fn query_graph(
            &self,
            _query: &serde_json::Value,
        ) -> Result<serde_json::Value, pylon_http::DataError> {
            Ok(serde_json::json!({}))
        }
        fn transact(
            &self,
            _ops: &[serde_json::Value],
        ) -> Result<(bool, Vec<serde_json::Value>), pylon_http::DataError> {
            Ok((true, Vec::new()))
        }
    }

    macro_rules! stub_ops {
        ($name:ident, $trait:path) => {
            struct $name;
        };
    }

    stub_ops!(StubRooms, RoomOps);
    stub_ops!(StubCache, CacheOps);
    stub_ops!(StubPubSub, PubSubOps);
    stub_ops!(StubJobs, JobOps);
    stub_ops!(StubScheduler, SchedulerOps);
    stub_ops!(StubWorkflows, WorkflowOps);
    stub_ops!(StubFiles, FileOps);
    stub_ops!(StubOpenApi, OpenApiGenerator);
    stub_ops!(StubEmail, EmailSender);

    impl RoomOps for StubRooms {
        fn join(
            &self,
            _room: &str,
            _user_id: &str,
            _data: Option<serde_json::Value>,
        ) -> Result<(serde_json::Value, serde_json::Value), pylon_http::DataError> {
            Ok((serde_json::json!({}), serde_json::json!({})))
        }
        fn leave(&self, _room: &str, _user_id: &str) -> Option<serde_json::Value> {
            None
        }
        fn set_presence(
            &self,
            _room: &str,
            _user_id: &str,
            _data: serde_json::Value,
        ) -> Option<serde_json::Value> {
            None
        }
        fn broadcast(
            &self,
            _room: &str,
            _sender: Option<&str>,
            _topic: &str,
            _data: serde_json::Value,
        ) -> Option<serde_json::Value> {
            None
        }
        fn list_rooms(&self) -> Vec<String> {
            vec![]
        }
        fn room_size(&self, _name: &str) -> usize {
            0
        }
        fn members(&self, _name: &str) -> Vec<serde_json::Value> {
            vec![]
        }
    }
    impl CacheOps for StubCache {
        fn handle_command(&self, _body: &str) -> (u16, String) {
            (200, "{}".into())
        }
        fn handle_get(&self, _key: &str) -> (u16, String) {
            (404, "{}".into())
        }
        fn handle_delete(&self, _key: &str) -> (u16, String) {
            (200, "{}".into())
        }
    }
    impl PubSubOps for StubPubSub {
        fn handle_publish(&self, _body: &str) -> (u16, String) {
            (200, "{}".into())
        }
        fn handle_channels(&self) -> (u16, String) {
            (200, "[]".into())
        }
        fn handle_history(&self, _channel: &str, _url: &str) -> (u16, String) {
            (200, "[]".into())
        }
    }
    impl JobOps for StubJobs {
        fn enqueue(
            &self,
            _name: &str,
            _payload: serde_json::Value,
            _priority: &str,
            _delay_secs: u64,
            _max_retries: u32,
            _queue: &str,
        ) -> String {
            "job-id".into()
        }
        fn stats(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn dead_letters(&self) -> serde_json::Value {
            serde_json::json!([])
        }
        fn retry_dead(&self, _id: &str) -> bool {
            false
        }
        fn list_jobs(
            &self,
            _status: Option<&str>,
            _queue: Option<&str>,
            _limit: usize,
        ) -> serde_json::Value {
            serde_json::json!([])
        }
        fn get_job(&self, _id: &str) -> Option<serde_json::Value> {
            None
        }
    }
    impl SchedulerOps for StubScheduler {
        fn list_tasks(&self) -> serde_json::Value {
            serde_json::json!([])
        }
        fn trigger(&self, _name: &str) -> bool {
            false
        }
    }
    impl WorkflowOps for StubWorkflows {
        fn definitions(&self) -> serde_json::Value {
            serde_json::json!([])
        }
        fn start(&self, _name: &str, _input: serde_json::Value) -> Result<String, String> {
            Ok("wf-id".into())
        }
        fn list(&self, _status: Option<&str>) -> serde_json::Value {
            serde_json::json!([])
        }
        fn get(&self, _id: &str) -> Option<serde_json::Value> {
            None
        }
        fn advance(&self, _id: &str) -> Result<String, String> {
            Ok("running".into())
        }
        fn send_event(
            &self,
            _id: &str,
            _event: &str,
            _data: serde_json::Value,
        ) -> Result<(), String> {
            Ok(())
        }
        fn cancel(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }
    }
    impl FileOps for StubFiles {
        fn upload(&self, _body: &str) -> (u16, String) {
            (501, "{}".into())
        }
        fn get_file(
            &self,
            _id: &str,
            _requester_user_id: Option<&str>,
            _is_admin: bool,
        ) -> (u16, String) {
            (404, "{}".into())
        }
    }
    impl OpenApiGenerator for StubOpenApi {
        fn generate(&self, _base: &str) -> String {
            "{}".into()
        }
    }
    impl EmailSender for StubEmail {
        fn send(&self, _to: &str, _subject: &str, _body: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn empty_manifest() -> AppManifest {
        AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
        }
    }

    /// Scaffold a RouterContext for tests. Caller chooses is_dev + auth.
    fn with_ctx<F>(is_dev: bool, auth: &AuthContext, f: F)
    where
        F: FnOnce(&RouterContext),
    {
        with_ctx_hooks(is_dev, auth, &NoopPluginHooks, f);
    }

    /// Variant of `with_ctx` that takes an explicit CookieConfig. Lets
    /// cookie-shape regression tests avoid `std::env::set_var` races
    /// against the rest of the test binary.
    fn with_ctx_cookie<F>(is_dev: bool, auth: &AuthContext, cookie_config: CookieConfig, f: F)
    where
        F: FnOnce(&RouterContext),
    {
        with_ctx_full(is_dev, auth, &NoopPluginHooks, Some(cookie_config), None, f);
    }

    /// Variant of `with_ctx` that takes an explicit ChangeNotifier.
    /// Used by tests asserting that auth surfaces (select-org,
    /// session revoke, ...) push the session-changed envelope.
    fn with_ctx_notifier<F>(
        is_dev: bool,
        auth: &AuthContext,
        notifier: &dyn ChangeNotifier,
        f: F,
    ) where
        F: FnOnce(&RouterContext),
    {
        with_ctx_full(is_dev, auth, &NoopPluginHooks, None, Some(notifier), f);
    }

    fn with_ctx_hooks<F>(is_dev: bool, auth: &AuthContext, hooks: &dyn PluginHookOps, f: F)
    where
        F: FnOnce(&RouterContext),
    {
        with_ctx_full(is_dev, auth, hooks, None, None, f);
    }

    fn with_ctx_full<F>(
        is_dev: bool,
        auth: &AuthContext,
        hooks: &dyn PluginHookOps,
        cookie_config_override: Option<CookieConfig>,
        notifier_override: Option<&dyn ChangeNotifier>,
        f: F,
    ) where
        F: FnOnce(&RouterContext),
    {
        let manifest = empty_manifest();
        let store = StubDataStore {
            manifest: manifest.clone(),
        };
        let session_store = SessionStore::new();
        let magic_codes = MagicCodeStore::new();
        let oauth_state = OAuthStateStore::new();
        let account_store = pylon_auth::AccountStore::new();
        let api_keys = pylon_auth::api_key::ApiKeyStore::new();
        // OrgStore reads/writes via DataStore. Tests share the
        // routing-context stub.
        let orgs_store: std::sync::Arc<dyn pylon_http::DataStore> =
            std::sync::Arc::new(StubDataStore {
                manifest: manifest.clone(),
            });
        let orgs = pylon_auth::org::OrgStore::new(orgs_store, manifest.auth.org.clone());
        let siwe = pylon_auth::siwe::NonceStore::new();
        let phone_codes = pylon_auth::phone::PhoneCodeStore::new();
        let passkeys = pylon_auth::webauthn::PasskeyStore::new();
        let verification = pylon_auth::verification::VerificationStore::new();
        let audit = pylon_auth::audit::AuditStore::new();
        let trusted_devices = pylon_auth::trusted_device::InMemoryTrustedDeviceStore::new();
        let org_sso = pylon_auth::org_sso::InMemoryOrgSsoStore::new();
        let saml = pylon_auth::saml::InMemorySamlStore::new();
        let policy_engine = PolicyEngine::from_manifest(&manifest);
        let change_log = ChangeLog::new();
        let default_notifier = NoopNotifier;
        let notifier: &dyn ChangeNotifier =
            notifier_override.unwrap_or(&default_notifier);
        let rooms = StubRooms;
        let cache = StubCache;
        let pubsub = StubPubSub;
        let jobs = StubJobs;
        let scheduler = StubScheduler;
        let workflows = StubWorkflows;
        let files = StubFiles;
        let openapi = StubOpenApi;
        let email = StubEmail;
        let cookie_config = cookie_config_override
            .unwrap_or_else(|| CookieConfig::from_env(&CookieConfig::default_name_for("test")));

        let ctx = RouterContext {
            store: &store,
            session_store: &session_store,
            magic_codes: &magic_codes,
            oauth_state: &oauth_state,
            account_store: &account_store,
            api_keys: &api_keys,
            orgs: &orgs,
            siwe: &siwe,
            phone_codes: &phone_codes,
            passkeys: &passkeys,
            verification: &verification,
            audit: &audit,
            trusted_devices: &trusted_devices,
            org_sso: &org_sso,
            saml: &saml,
            trusted_origins: &[],
            policy_engine: &policy_engine,
            change_log: &change_log,
            notifier,
            rooms: &rooms,
            cache: &cache,
            pubsub: &pubsub,
            jobs: &jobs,
            scheduler: &scheduler,
            workflows: &workflows,
            files: &files,
            openapi: &openapi,
            functions: None,
            email: &email,
            shards: None,
            plugin_hooks: hooks,
            auth_ctx: auth,
            is_dev,
            request_headers: &[],
            peer_ip: "127.0.0.1",
            cookie_config: &cookie_config,
            response_headers: RefCell::new(Vec::new()),
        };
        f(&ctx);
    }

    // -----------------------------------------------------------------------
    // Regression tests for the previously-vulnerable routes
    // -----------------------------------------------------------------------

    /// Prior vuln: POST /api/auth/session accepted any user_id from anonymous
    /// callers and minted a valid session. Now: prod requires admin.
    #[test]
    fn auth_session_refuses_non_admin_in_prod() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/session",
                r#"{"user_id":"victim"}"#,
                None,
            );
            assert_eq!(status, 403);
            assert!(body.contains("FORBIDDEN"));
        });
    }

    #[test]
    fn auth_session_allowed_for_admin_in_prod() {
        let admin = AuthContext::admin();
        with_ctx(false, &admin, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/session",
                r#"{"user_id":"alice"}"#,
                None,
            );
            assert_eq!(status, 201);
        });
    }

    /// /api/auth/native-session refuses anonymous callers — there's
    /// no cookie session to mint a token for. Don't accidentally
    /// regress to the pre-fix /api/auth/session shape that minted
    /// for any user_id.
    #[test]
    fn auth_native_session_refuses_anonymous() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, body, _ct) =
                route(ctx, HttpMethod::Post, "/api/auth/native-session", "", None);
            assert_eq!(status, 401);
            assert!(body.contains("AUTH_REQUIRED"));
        });
    }

    /// /api/auth/native-session refuses API-key contexts. A leaked
    /// `pk.*` API key for some user must NOT be able to mint a real
    /// session — sessions work on every auth surface including
    /// routes that explicitly refuse API keys. Regression for the
    /// 2026-05-09 codex security audit.
    #[test]
    fn auth_native_session_refuses_api_key_auth() {
        let api_key_ctx =
            AuthContext::from_api_key("alice".to_string(), "key_abc".to_string(), None);
        with_ctx(false, &api_key_ctx, |ctx| {
            let (status, body, _ct) =
                route(ctx, HttpMethod::Post, "/api/auth/native-session", "", None);
            assert_eq!(status, 403);
            assert!(body.contains("API_KEY_AUTH_FORBIDDEN"));
        });
    }

    /// /api/auth/native-session mints a real SessionStore entry for
    /// the cookie-authed user — no admin gate, no privilege escalation
    /// (it can ONLY mint for the SAME user the cookie already
    /// represents). The minted token is the one native apps use for
    /// both HTTP `Authorization: Bearer <token>` and the WebSocket
    /// `bearer.<token>` subprotocol.
    #[test]
    fn auth_native_session_mints_for_cookie_user_in_prod() {
        let user = AuthContext::authenticated("alice".to_string());
        with_ctx(false, &user, |ctx| {
            let (status, body, _ct) =
                route(ctx, HttpMethod::Post, "/api/auth/native-session", "", None);
            assert_eq!(status, 201);
            // Body shape: { token, user_id, expires_at }
            assert!(body.contains("\"token\""));
            assert!(body.contains("\"alice\""));
            assert!(body.contains("\"expires_at\""));
            // Confirm the minted token resolves through SessionStore
            // (the WS path's auth check uses the same lookup) — this
            // is what closes the JWT-on-WS gap.
            let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
            let token = parsed["token"].as_str().unwrap();
            let resolved = ctx.session_store.get(token);
            assert!(
                resolved.is_some(),
                "minted token must resolve via SessionStore"
            );
            assert_eq!(resolved.unwrap().user_id, "alice");
        });
    }

    /// Prior vuln: OAuth callback accepted `{email, state}` without a real
    /// authorization code, letting anyone mint a session for any email.
    /// Now: prod requires an authorization code.
    #[test]
    fn oauth_callback_refuses_missing_code_in_prod() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            // Mint a state token so the state check passes — the `code`
            // requirement is what must still stop us.
            let state = ctx
                .oauth_state
                .create("google", "https://app/cb", "https://app/cb");
            let body = format!(r#"{{"state":"{state}","email":"victim@example.com"}}"#);
            let (status, resp, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/callback/google",
                &body,
                None,
            );
            assert_eq!(status, 400);
            assert!(resp.contains("authorization") || resp.contains("code"));
        });
    }

    /// Prior vuln: /api/sync/push had no auth check.
    #[test]
    fn sync_push_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/sync/push",
                r#"{"changes":[]}"#,
                None,
            );
            assert_eq!(status, 403);
            assert!(body.contains("FORBIDDEN"));
        });
    }

    /// Prior vuln: /api/transact had no auth check.
    #[test]
    fn transact_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/transact",
                r#"{"ops":[]}"#,
                None,
            );
            assert_eq!(status, 403);
        });
    }

    /// Prior vuln: /api/workflows/start had no auth check.
    #[test]
    fn workflow_start_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/workflows/start",
                r#"{"name":"x"}"#,
                None,
            );
            assert_eq!(status, 403);
        });
    }

    /// Prior vuln: /api/jobs enqueue was open.
    #[test]
    fn jobs_enqueue_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/jobs",
                r#"{"name":"x","payload":{}}"#,
                None,
            );
            assert_eq!(status, 403);
        });
    }

    // -----------------------------------------------------------------------
    // Robustness / fuzz-style property tests
    //
    // The router is the one public entry point that takes arbitrary bytes
    // from the network. Any input must yield a response, not a panic.
    // These tests hammer the handlers with malformed bodies, weird paths,
    // and deeply-nested JSON. If any of them panic or loop, we catch it
    // here instead of in production.
    // -----------------------------------------------------------------------

    fn assert_route_doesnt_panic(ctx: &RouterContext, method: HttpMethod, url: &str, body: &str) {
        // `route` is synchronous. A panic would abort the test thread; the
        // test harness would fail the whole test. So just call it — success
        // means no panic.
        let (_status, _body, _ct) = route(ctx, method, url, body, None);
    }

    #[test]
    fn fuzz_malformed_json_bodies_never_panic() {
        let admin = AuthContext::admin();
        with_ctx(true, &admin, |ctx| {
            let samples = [
                "",
                "not json",
                "{",
                "}",
                "{\"",
                "{\"key\":",
                "[]",
                "null",
                "true",
                "\"string\"",
                "{\"changes\":\"not an array\"}",
                &format!("{{\"deeply\":{}}}", "{".repeat(1000)),
                "{\"unicode\":\"\\u0000\"}",
                "{\"numbers\":1e308}",
                "{\"negative\":-999999999999999}",
            ];
            for body in &samples {
                for url in &[
                    "/api/sync/push",
                    "/api/transact",
                    "/api/import",
                    "/api/batch",
                    "/api/jobs",
                    "/api/auth/session",
                    "/api/auth/magic/send",
                ] {
                    assert_route_doesnt_panic(ctx, HttpMethod::Post, url, body);
                }
            }
        });
    }

    #[test]
    fn fuzz_weird_urls_never_panic() {
        let admin = AuthContext::admin();
        with_ctx(true, &admin, |ctx| {
            let samples = [
                "/",
                "/api",
                "/api/",
                "/api/entities/",
                "/api/entities//",
                "/api/entities/%00",
                "/api/entities/../escape",
                "/api/entities/User?garbage=\x01",
                "/api/entities/User?$limit=abc&$order=garbage",
                &format!("/api/entities/{}", "a".repeat(10_000)),
                "/api/fn/",
                "/api/fn/traces",
                "/api/shards/id/connect",
                "/api/workflows/definitions",
                "/api/workflows/nonexistent/advance",
                "/api/rooms/",
                "/api/rooms/%20",
            ];
            for url in &samples {
                assert_route_doesnt_panic(ctx, HttpMethod::Get, url, "");
                assert_route_doesnt_panic(ctx, HttpMethod::Post, url, "{}");
                assert_route_doesnt_panic(ctx, HttpMethod::Delete, url, "");
            }
        });
    }

    #[test]
    fn fuzz_deeply_nested_json_dont_stack_overflow() {
        // serde_json has an internal recursion limit (default 128); confirm
        // depths beyond that return 400 rather than overflow the stack.
        let admin = AuthContext::admin();
        with_ctx(true, &admin, |ctx| {
            let depth = 300;
            let body = format!("{}{}", "[".repeat(depth), "]".repeat(depth),);
            let (status, _body, _ct) = route(ctx, HttpMethod::Post, "/api/sync/push", &body, None);
            // Serde may reject with 400, or the handler may accept and
            // treat as empty — either is fine. The key property: no panic.
            assert!(status >= 200 && status < 600);
        });
    }

    /// Regression: gate respects PYLON_REQUIRE_EMAIL_VERIFICATION env,
    /// blocks non-/api/auth/* for unverified users, lets /api/auth/* +
    /// /api/manifest pass, and bypasses for admin / anonymous.
    ///
    /// Tests the gate function directly because route() also hits other
    /// auth checks on the test routes, making end-to-end status codes
    /// noisy. Direct gate testing is more precise.
    #[test]
    fn email_verification_gate_behavior() {
        let user = AuthContext {
            user_id: Some("u-1".into()),
            is_admin: false,
            is_guest: false,
            roles: vec![],
            tenant_id: None,
            api_key_id: None,
            api_key_scopes: None,
            is_trusted_device: false,
        };
        let admin = AuthContext::admin();
        let anon = AuthContext::anonymous();

        // env unset: gate never fires
        std::env::remove_var("PYLON_REQUIRE_EMAIL_VERIFICATION");
        with_ctx(true, &user, |ctx| {
            assert!(email_verification_gate(ctx, "/api/fn").is_none());
        });

        // env set, unverified user, non-auth route: blocked
        std::env::set_var("PYLON_REQUIRE_EMAIL_VERIFICATION", "1");
        with_ctx(true, &user, |ctx| {
            let r = email_verification_gate(ctx, "/api/fn");
            assert!(r.is_some(), "gate must trigger for unverified user");
            let (status, body) = r.unwrap();
            assert_eq!(status, 403);
            assert!(body.contains("EMAIL_NOT_VERIFIED"));
        });

        // env set, unverified user, /api/auth/me: bypassed
        with_ctx(true, &user, |ctx| {
            assert!(
                email_verification_gate(ctx, "/api/auth/me").is_none(),
                "/api/auth/me must bypass"
            );
            assert!(
                email_verification_gate(ctx, "/api/auth/email/send-verification").is_none(),
                "/api/auth/email/send-verification must bypass"
            );
            assert!(
                email_verification_gate(ctx, "/api/manifest").is_none(),
                "/api/manifest must bypass (chrome needs it to render)"
            );
        });

        // env set, admin: bypassed
        with_ctx(true, &admin, |ctx| {
            assert!(email_verification_gate(ctx, "/api/fn").is_none());
        });

        // env set, anonymous: bypassed (nothing to verify)
        with_ctx(true, &anon, |ctx| {
            assert!(email_verification_gate(ctx, "/api/fn").is_none());
        });

        std::env::remove_var("PYLON_REQUIRE_EMAIL_VERIFICATION");
    }

    #[test]
    fn fuzz_unusual_http_methods_gracefully() {
        let admin = AuthContext::admin();
        with_ctx(true, &admin, |ctx| {
            for method in [
                HttpMethod::Get,
                HttpMethod::Post,
                HttpMethod::Put,
                HttpMethod::Patch,
                HttpMethod::Delete,
                HttpMethod::Options,
                HttpMethod::Head,
            ] {
                let (_status, _body, _ct) = route(ctx, method, "/api/entities/User", "{}", None);
            }
        });
    }

    // -----------------------------------------------------------------------
    // /api/auth/email/* — verify the auth gate, the rate limiter, and a
    // happy-path send→verify cycle. Uses a User-aware stub store because
    // the default StubDataStore returns None for every get_by_id.
    // -----------------------------------------------------------------------

    /// Stub store that pretends User "u-1" exists with email
    /// "alice@example.com" and tracks update calls so we can assert the
    /// emailVerified field gets set.
    struct UserStubStore {
        manifest: AppManifest,
        last_update: std::sync::Mutex<Option<(String, String, serde_json::Value)>>,
    }
    impl pylon_http::DataStore for UserStubStore {
        fn manifest(&self) -> &AppManifest {
            &self.manifest
        }
        fn insert(
            &self,
            _e: &str,
            _d: &serde_json::Value,
        ) -> Result<String, pylon_http::DataError> {
            Ok("u-1".into())
        }
        fn get_by_id(
            &self,
            entity: &str,
            id: &str,
        ) -> Result<Option<serde_json::Value>, pylon_http::DataError> {
            if entity == "User" && id == "u-1" {
                return Ok(Some(serde_json::json!({
                    "id": "u-1",
                    "email": "alice@example.com",
                    "displayName": "Alice",
                })));
            }
            Ok(None)
        }
        fn list(&self, _e: &str) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
            Ok(vec![])
        }
        fn list_after(
            &self,
            _e: &str,
            _a: Option<&str>,
            _l: usize,
        ) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
            Ok(vec![])
        }
        fn update(
            &self,
            entity: &str,
            id: &str,
            data: &serde_json::Value,
        ) -> Result<bool, pylon_http::DataError> {
            *self.last_update.lock().unwrap() = Some((entity.into(), id.into(), data.clone()));
            Ok(true)
        }
        fn delete(&self, _e: &str, _i: &str) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn lookup(
            &self,
            _e: &str,
            _f: &str,
            _v: &str,
        ) -> Result<Option<serde_json::Value>, pylon_http::DataError> {
            Ok(None)
        }
        fn link(
            &self,
            _e: &str,
            _i: &str,
            _r: &str,
            _t: &str,
        ) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn unlink(&self, _e: &str, _i: &str, _r: &str) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn query_filtered(
            &self,
            _e: &str,
            _f: &serde_json::Value,
        ) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
            Ok(vec![])
        }
        fn query_graph(
            &self,
            _q: &serde_json::Value,
        ) -> Result<serde_json::Value, pylon_http::DataError> {
            Ok(serde_json::json!({}))
        }
        fn aggregate(
            &self,
            _e: &str,
            _s: &serde_json::Value,
        ) -> Result<serde_json::Value, pylon_http::DataError> {
            Ok(serde_json::json!({}))
        }
        fn transact(
            &self,
            _o: &[serde_json::Value],
        ) -> Result<(bool, Vec<serde_json::Value>), pylon_http::DataError> {
            Ok((true, vec![]))
        }
        fn search(
            &self,
            _e: &str,
            _q: &serde_json::Value,
        ) -> Result<serde_json::Value, pylon_http::DataError> {
            Ok(serde_json::json!({}))
        }
    }

    /// Capture-the-email stub so we can assert the body the user would
    /// have received. Production wiring does this through an Resend /
    /// SES adapter; tests just want to read what got "sent".
    struct CaptureEmail {
        sent: std::sync::Mutex<Vec<(String, String, String)>>,
    }
    impl EmailSender for CaptureEmail {
        fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap()
                .push((to.into(), subject.into(), body.into()));
            Ok(())
        }
    }

    fn with_user_ctx<F>(is_dev: bool, auth: &AuthContext, f: F)
    where
        F: FnOnce(&RouterContext, &UserStubStore, &CaptureEmail, &MagicCodeStore),
    {
        let manifest = empty_manifest();
        let store = UserStubStore {
            manifest: manifest.clone(),
            last_update: std::sync::Mutex::new(None),
        };
        let session_store = SessionStore::new();
        let magic_codes = MagicCodeStore::new();
        let oauth_state = OAuthStateStore::new();
        let account_store = pylon_auth::AccountStore::new();
        let api_keys = pylon_auth::api_key::ApiKeyStore::new();
        // OrgStore reads/writes via DataStore. Tests share the
        // routing-context stub.
        let orgs_store: std::sync::Arc<dyn pylon_http::DataStore> =
            std::sync::Arc::new(StubDataStore {
                manifest: manifest.clone(),
            });
        let orgs = pylon_auth::org::OrgStore::new(orgs_store, manifest.auth.org.clone());
        let siwe = pylon_auth::siwe::NonceStore::new();
        let phone_codes = pylon_auth::phone::PhoneCodeStore::new();
        let passkeys = pylon_auth::webauthn::PasskeyStore::new();
        let verification = pylon_auth::verification::VerificationStore::new();
        let audit = pylon_auth::audit::AuditStore::new();
        let trusted_devices = pylon_auth::trusted_device::InMemoryTrustedDeviceStore::new();
        let org_sso = pylon_auth::org_sso::InMemoryOrgSsoStore::new();
        let saml = pylon_auth::saml::InMemorySamlStore::new();
        let policy_engine = PolicyEngine::from_manifest(&manifest);
        let change_log = ChangeLog::new();
        let notifier = NoopNotifier;
        let rooms = StubRooms;
        let cache = StubCache;
        let pubsub = StubPubSub;
        let jobs = StubJobs;
        let scheduler = StubScheduler;
        let workflows = StubWorkflows;
        let files = StubFiles;
        let openapi = StubOpenApi;
        let email = CaptureEmail {
            sent: std::sync::Mutex::new(vec![]),
        };
        let hooks = NoopPluginHooks;
        let cookie_config = CookieConfig::from_env(&CookieConfig::default_name_for("test"));

        let ctx = RouterContext {
            store: &store,
            session_store: &session_store,
            magic_codes: &magic_codes,
            oauth_state: &oauth_state,
            account_store: &account_store,
            api_keys: &api_keys,
            orgs: &orgs,
            siwe: &siwe,
            phone_codes: &phone_codes,
            passkeys: &passkeys,
            verification: &verification,
            audit: &audit,
            trusted_devices: &trusted_devices,
            org_sso: &org_sso,
            saml: &saml,
            trusted_origins: &[],
            policy_engine: &policy_engine,
            change_log: &change_log,
            notifier: &notifier,
            rooms: &rooms,
            cache: &cache,
            pubsub: &pubsub,
            jobs: &jobs,
            scheduler: &scheduler,
            workflows: &workflows,
            files: &files,
            openapi: &openapi,
            functions: None,
            email: &email,
            shards: None,
            plugin_hooks: &hooks,
            auth_ctx: auth,
            is_dev,
            request_headers: &[],
            peer_ip: "127.0.0.1",
            cookie_config: &cookie_config,
            response_headers: RefCell::new(Vec::new()),
        };
        f(&ctx, &store, &email, &magic_codes);
    }

    #[test]
    fn email_send_verification_requires_auth() {
        let anon = AuthContext::anonymous();
        with_user_ctx(true, &anon, |ctx, _, _, _| {
            let (status, body, _) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/email/send-verification",
                "{}",
                None,
            );
            assert_eq!(status, 401);
            assert!(body.contains("UNAUTHORIZED"));
        });
    }

    #[test]
    fn email_verify_requires_auth() {
        let anon = AuthContext::anonymous();
        with_user_ctx(true, &anon, |ctx, _, _, _| {
            let (status, body, _) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/email/verify",
                r#"{"code":"123456"}"#,
                None,
            );
            assert_eq!(status, 401);
            assert!(body.contains("UNAUTHORIZED"));
        });
    }

    #[test]
    fn email_send_verification_uses_session_email_not_body() {
        // Caller is "u-1" (alice@example.com). Even if they put a
        // different email in the body, the code should be issued for
        // the SESSION's email — otherwise an authed caller could spam
        // codes to arbitrary addresses.
        let alice = AuthContext::authenticated("u-1".into());
        with_user_ctx(true, &alice, |ctx, _, email, _| {
            let (status, body, _) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/email/send-verification",
                r#"{"email":"victim@example.com"}"#,
                None,
            );
            assert_eq!(status, 200);
            // Dev mode echoes the code; verify the recipient is alice,
            // not the body's victim.
            let sent = email.sent.lock().unwrap();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].0, "alice@example.com");
            assert!(body.contains("alice@example.com"));
            assert!(!body.contains("victim@example.com"));
        });
    }

    #[test]
    fn email_verify_happy_path_stamps_email_verified() {
        let alice = AuthContext::authenticated("u-1".into());
        with_user_ctx(true, &alice, |ctx, store, _, magic_codes| {
            // Pre-issue a code (skipping the send endpoint) so we test
            // verify in isolation.
            let code = magic_codes.try_create("alice@example.com").unwrap();
            let body = format!(r#"{{"code":"{code}"}}"#);
            let (status, resp, _) =
                route(ctx, HttpMethod::Post, "/api/auth/email/verify", &body, None);
            assert_eq!(status, 200);
            assert!(resp.contains("\"verified\":true"));
            // Update was attempted on User u-1 with emailVerified set.
            let last = store.last_update.lock().unwrap();
            let (entity, id, data) = last.as_ref().expect("update should have fired");
            assert_eq!(entity, "User");
            assert_eq!(id, "u-1");
            assert!(data.get("emailVerified").is_some());
        });
    }

    #[test]
    fn email_verify_rejects_wrong_code() {
        let alice = AuthContext::authenticated("u-1".into());
        with_user_ctx(true, &alice, |ctx, store, _, magic_codes| {
            let _ = magic_codes.try_create("alice@example.com").unwrap();
            let (status, body, _) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/email/verify",
                r#"{"code":"999999"}"#,
                None,
            );
            assert_eq!(status, 401);
            assert!(body.contains("INVALID_CODE"));
            // No update should have happened.
            assert!(store.last_update.lock().unwrap().is_none());
        });
    }

    /// Dev mode keeps the old permissive behaviour for local tooling.
    #[test]
    fn auth_session_allowed_in_dev_mode() {
        let anon = AuthContext::anonymous();
        with_ctx(true, &anon, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/session",
                r#"{"user_id":"alice"}"#,
                None,
            );
            assert_eq!(status, 201);
        });
    }

    // -----------------------------------------------------------------------
    // Plugin CRUD hook wiring — prior vuln: POST/PATCH/DELETE on
    // /api/entities/* bypassed the registered plugin before_/after_ hooks,
    // so validation/audit_log/webhooks/slugify/timestamps never saw data-
    // plane writes. These tests pin that the router now runs them.
    // -----------------------------------------------------------------------

    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    struct CountingHooks {
        before_insert_calls: AtomicU32,
        after_insert_calls: AtomicU32,
        before_delete_calls: AtomicU32,
        reject_on_entity: Option<&'static str>,
    }

    impl CountingHooks {
        fn new() -> Self {
            Self {
                before_insert_calls: AtomicU32::new(0),
                after_insert_calls: AtomicU32::new(0),
                before_delete_calls: AtomicU32::new(0),
                reject_on_entity: None,
            }
        }
    }

    impl PluginHookOps for CountingHooks {
        fn before_insert(
            &self,
            entity: &str,
            _data: &mut serde_json::Value,
            _auth: &AuthContext,
        ) -> Result<(), (u16, String, String)> {
            self.before_insert_calls.fetch_add(1, Ordering::SeqCst);
            if self.reject_on_entity == Some(entity) {
                return Err((422, "VALIDATION".into(), "rejected by plugin".into()));
            }
            Ok(())
        }
        fn after_insert(
            &self,
            _entity: &str,
            _id: &str,
            _data: &serde_json::Value,
            _auth: &AuthContext,
        ) {
            self.after_insert_calls.fetch_add(1, Ordering::SeqCst);
        }
        fn before_update(
            &self,
            _entity: &str,
            _id: &str,
            _data: &mut serde_json::Value,
            _auth: &AuthContext,
        ) -> Result<(), (u16, String, String)> {
            Ok(())
        }
        fn after_update(
            &self,
            _entity: &str,
            _id: &str,
            _data: &serde_json::Value,
            _auth: &AuthContext,
        ) {
        }
        fn before_delete(
            &self,
            _entity: &str,
            _id: &str,
            _auth: &AuthContext,
        ) -> Result<(), (u16, String, String)> {
            self.before_delete_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn after_delete(&self, _entity: &str, _id: &str, _auth: &AuthContext) {}
    }

    #[test]
    fn plugin_hooks_fire_on_entity_post() {
        // StubDataStore::insert always succeeds with id="stub-id", so we
        // just assert the before_/after_ counters tick.
        let admin = AuthContext::admin();
        let hooks = CountingHooks::new();
        with_ctx_hooks(true, &admin, &hooks, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/entities/User",
                r#"{"email":"a@b"}"#,
                None,
            );
            assert_eq!(status, 201);
        });
        assert_eq!(hooks.before_insert_calls.load(Ordering::SeqCst), 1);
        assert_eq!(hooks.after_insert_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn plugin_before_insert_rejection_short_circuits_write() {
        // If the plugin rejects, the store's insert must NOT be called and
        // the status must propagate.
        let admin = AuthContext::admin();
        let rejector = CountingHooks {
            reject_on_entity: Some("User"),
            ..CountingHooks::new()
        };
        with_ctx_hooks(true, &admin, &rejector, |ctx| {
            let (status, body, _ct) =
                route(ctx, HttpMethod::Post, "/api/entities/User", r#"{}"#, None);
            assert_eq!(status, 422);
            assert!(body.contains("VALIDATION"));
        });
        assert_eq!(rejector.before_insert_calls.load(Ordering::SeqCst), 1);
        // after_insert must NOT have been called when before_insert rejected.
        assert_eq!(rejector.after_insert_calls.load(Ordering::SeqCst), 0);
    }

    // -----------------------------------------------------------------------
    // GDPR export + purge — pinned behavior so accidental breakage shows up
    // as a test failure instead of a compliance surprise.
    // -----------------------------------------------------------------------

    #[test]
    fn gdpr_export_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/admin/users/alice/export",
                "",
                None,
            );
            assert_eq!(status, 403);
            assert!(body.contains("FORBIDDEN"));
        });
    }

    #[test]
    fn gdpr_purge_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Delete,
                "/api/admin/users/alice/purge",
                "",
                None,
            );
            assert_eq!(status, 403);
        });
    }

    #[test]
    fn gdpr_export_returns_envelope_for_admin() {
        let admin = AuthContext::admin();
        with_ctx(true, &admin, |ctx| {
            let (status, body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/admin/users/alice/export",
                "",
                None,
            );
            assert_eq!(status, 200);
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["user_id"], "alice");
            assert!(v["entities"].is_object());
            assert!(v["exported_at"].is_string());
        });
    }

    #[test]
    fn plugin_hooks_fire_on_entity_delete() {
        let admin = AuthContext::admin();
        let hooks = CountingHooks::new();
        with_ctx_hooks(true, &admin, &hooks, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Delete,
                "/api/entities/User/stub-id",
                "",
                None,
            );
            assert_eq!(status, 200);
        });
        assert_eq!(hooks.before_delete_calls.load(Ordering::SeqCst), 1);
    }

    // -----------------------------------------------------------------------
    // Webhook endpoint — /api/webhooks/:action
    //
    // No functions are registered in the stub RouterContext, so we can only
    // assert the error paths (FUNCTIONS_NOT_AVAILABLE, FN_NOT_FOUND). The
    // happy-path with request-context propagation is covered by the TS
    // runtime tests + a manual integration check against a live server.
    // -----------------------------------------------------------------------

    #[test]
    fn webhook_returns_503_when_functions_unavailable() {
        let anon = AuthContext::anonymous();
        with_ctx(true, &anon, |ctx| {
            let (status, body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/webhooks/stripe_handler",
                "{}",
                None,
            );
            assert_eq!(status, 503);
            assert!(body.contains("FUNCTIONS_NOT_AVAILABLE"));
        });
    }

    #[test]
    fn webhook_accepts_any_http_method() {
        // Regression: providers send GET challenge requests (e.g. Slack URL
        // verification). The endpoint must route regardless of method.
        let anon = AuthContext::anonymous();
        with_ctx(true, &anon, |ctx| {
            for method in [
                HttpMethod::Get,
                HttpMethod::Post,
                HttpMethod::Put,
                HttpMethod::Patch,
                HttpMethod::Delete,
            ] {
                let (status, _body, _ct) = route(ctx, method, "/api/webhooks/any_name", "", None);
                // Without function runtime it's 503 regardless; the point
                // is that we didn't 405 Method Not Allowed.
                assert_ne!(status, 405);
            }
        });
    }

    // -----------------------------------------------------------------------
    // Auth matrix regression scaffold
    //
    // Goal: catch reviewer-class bugs at PR time. Every route in this
    // table is hit as anonymous, guest, and authed-non-admin. The
    // EXPECTED column is what the route should return for that
    // identity. Adding a new route? Add a row. Discovering a bypass?
    // Add the regression here so it stays caught.
    //
    // Status semantics:
    //   401 = AUTH_REQUIRED            (require_auth)
    //   403 = FORBIDDEN | POLICY_DENIED (require_admin / policy)
    //   any = 200..=599 means "anything but 405" — for routes that
    //         legitimately reach the handler with this identity and
    //         may 200/400/etc. depending on body.
    // -----------------------------------------------------------------------

    #[derive(Clone, Copy)]
    enum Expect {
        /// Status must equal this value.
        Eq(u16),
        /// Status must be 401 or 403 (rejected before handler logic).
        Rejected,
        /// Anything except 405 — handler reached, body validation may
        /// have triggered other errors.
        ReachedHandler,
    }

    fn assert_expect(actual: u16, want: Expect, label: &str) {
        match want {
            Expect::Eq(s) => assert_eq!(actual, s, "{label}: expected status {s}, got {actual}"),
            Expect::Rejected => assert!(
                actual == 401 || actual == 403,
                "{label}: expected 401 or 403, got {actual}"
            ),
            Expect::ReachedHandler => assert_ne!(
                actual, 405,
                "{label}: route should accept this method, got 405"
            ),
        }
    }

    /// Hit a route as anon / guest / authed-non-admin and assert each
    /// identity gets the documented response. Catches reviewer-class
    /// bugs (e.g. a P1 finding that an endpoint is_admin-gated drifts
    /// to public during a refactor).
    fn matrix_check(
        method: HttpMethod,
        url: &str,
        body: &str,
        expect_anon: Expect,
        expect_guest: Expect,
        expect_user: Expect,
    ) {
        let anon = AuthContext::anonymous();
        let guest = AuthContext::guest("guest-1".into());
        let user = AuthContext::authenticated("u-1".into());

        for (auth, want, who) in [
            (&anon, expect_anon, "anon"),
            (&guest, expect_guest, "guest"),
            (&user, expect_user, "user"),
        ] {
            with_ctx(false, auth, |ctx| {
                let (status, _body, _ct) = route(ctx, method, url, body, None);
                assert_expect(status, want, &format!("{who} {method:?} {url}"));
            });
        }
    }

    /// Like matrix_check, but the test scaffold loads a manifest with
    /// a deny-by-default policy on the named entity. Use this for
    /// policy-gated routes (cursor, filtered query, CRDT push) where
    /// the gate's job is "call check_entity_*" — without a denying
    /// policy in scope the call would silently pass.
    fn matrix_check_with_deny_policy(
        deny_entity: &str,
        method: HttpMethod,
        url: &str,
        body: &str,
        expect_anon: Expect,
        expect_guest: Expect,
        expect_user: Expect,
    ) {
        use pylon_kernel::{AppManifest, ManifestPolicy, MANIFEST_VERSION};
        let anon = AuthContext::anonymous();
        let guest = AuthContext::guest("guest-1".into());
        let user = AuthContext::authenticated("u-1".into());

        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![ManifestPolicy {
                name: "denyAll".into(),
                entity: Some(deny_entity.into()),
                allow_read: Some("false".into()),
                allow_update: Some("false".into()),
                ..Default::default()
            }],
            auth: Default::default(),
        };
        let store = StubDataStore {
            manifest: manifest.clone(),
        };
        let session_store = SessionStore::new();
        let magic_codes = MagicCodeStore::new();
        let oauth_state = OAuthStateStore::new();
        let account_store = pylon_auth::AccountStore::new();
        let api_keys = pylon_auth::api_key::ApiKeyStore::new();
        // OrgStore reads/writes via DataStore. Tests share the
        // routing-context stub.
        let orgs_store: std::sync::Arc<dyn pylon_http::DataStore> =
            std::sync::Arc::new(StubDataStore {
                manifest: manifest.clone(),
            });
        let orgs = pylon_auth::org::OrgStore::new(orgs_store, manifest.auth.org.clone());
        let siwe = pylon_auth::siwe::NonceStore::new();
        let phone_codes = pylon_auth::phone::PhoneCodeStore::new();
        let passkeys = pylon_auth::webauthn::PasskeyStore::new();
        let verification = pylon_auth::verification::VerificationStore::new();
        let audit = pylon_auth::audit::AuditStore::new();
        let trusted_devices = pylon_auth::trusted_device::InMemoryTrustedDeviceStore::new();
        let org_sso = pylon_auth::org_sso::InMemoryOrgSsoStore::new();
        let saml = pylon_auth::saml::InMemorySamlStore::new();
        let policy_engine = PolicyEngine::from_manifest(&manifest);
        let change_log = ChangeLog::new();
        let notifier = NoopNotifier;
        let rooms = StubRooms;
        let cache = StubCache;
        let pubsub = StubPubSub;
        let jobs = StubJobs;
        let scheduler = StubScheduler;
        let workflows = StubWorkflows;
        let files = StubFiles;
        let openapi = StubOpenApi;
        let email = StubEmail;
        let cookie_config = CookieConfig::from_env(&CookieConfig::default_name_for("test"));

        for (auth, want, who) in [
            (&anon, expect_anon, "anon"),
            (&guest, expect_guest, "guest"),
            (&user, expect_user, "user"),
        ] {
            let ctx = RouterContext {
                store: &store,
                session_store: &session_store,
                magic_codes: &magic_codes,
                oauth_state: &oauth_state,
                account_store: &account_store,
                api_keys: &api_keys,
                orgs: &orgs,
                siwe: &siwe,
                phone_codes: &phone_codes,
                passkeys: &passkeys,
                verification: &verification,
                audit: &audit,
                trusted_devices: &trusted_devices,
                org_sso: &org_sso,
                saml: &saml,
                trusted_origins: &[],
                policy_engine: &policy_engine,
                change_log: &change_log,
                notifier: &notifier,
                rooms: &rooms,
                cache: &cache,
                pubsub: &pubsub,
                jobs: &jobs,
                scheduler: &scheduler,
                workflows: &workflows,
                files: &files,
                openapi: &openapi,
                functions: None,
                email: &email,
                shards: None,
                plugin_hooks: &NoopPluginHooks,
                auth_ctx: auth,
                is_dev: false,
                request_headers: &[],
                peer_ip: "127.0.0.1",
                cookie_config: &cookie_config,
                response_headers: RefCell::new(Vec::new()),
            };
            let (status, _body, _ct) = route(&ctx, method, url, body, None);
            assert_expect(status, want, &format!("{who} {method:?} {url}"));
        }
    }

    #[test]
    fn matrix_cache_admin_only() {
        matrix_check(
            HttpMethod::Get,
            "/api/cache/anykey",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
        matrix_check(
            HttpMethod::Post,
            "/api/cache",
            r#"{"op":"get","key":"x"}"#,
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
        matrix_check(
            HttpMethod::Delete,
            "/api/cache/anykey",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
    }

    #[test]
    fn matrix_pubsub_admin_only() {
        matrix_check(
            HttpMethod::Post,
            "/api/pubsub/publish",
            r#"{"channel":"x","message":"y"}"#,
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
        matrix_check(
            HttpMethod::Get,
            "/api/pubsub/channels",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
        matrix_check(
            HttpMethod::Get,
            "/api/pubsub/history/some-channel",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
    }

    #[test]
    fn matrix_jobs_read_admin_only() {
        matrix_check(
            HttpMethod::Get,
            "/api/jobs/stats",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
        matrix_check(
            HttpMethod::Get,
            "/api/jobs/dead",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
        matrix_check(
            HttpMethod::Get,
            "/api/jobs",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
        matrix_check(
            HttpMethod::Get,
            "/api/jobs/some-job-id",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
    }

    #[test]
    fn matrix_workflows_read_admin_only() {
        matrix_check(
            HttpMethod::Get,
            "/api/workflows/definitions",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
        matrix_check(
            HttpMethod::Get,
            "/api/workflows",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
        matrix_check(
            HttpMethod::Get,
            "/api/workflows/some-id",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
    }

    #[test]
    fn matrix_files_download_requires_auth() {
        // Anon must not enumerate uploads via predictable file IDs.
        // Guest + user can — files use require_auth, not require_admin.
        matrix_check(
            HttpMethod::Get,
            "/api/files/some-file-id",
            "",
            Expect::Eq(401),
            Expect::ReachedHandler,
            Expect::ReachedHandler,
        );
    }

    #[test]
    fn matrix_crdt_push_respects_update_policy() {
        // Pre-fix: any session (incl. guest) could push a CRDT update
        // to any addressable row. Now: when the entity has an update
        // policy that denies, even authed non-admins are blocked. Anon
        // bounces at the require_auth gate before policy is consulted.
        matrix_check_with_deny_policy(
            "Doc",
            HttpMethod::Post,
            "/api/crdt/Doc/some-row",
            r#"{"update":"00"}"#,
            Expect::Eq(401),
            Expect::Rejected,
            Expect::Rejected,
        );
    }

    #[test]
    fn matrix_filtered_query_respects_read_policy() {
        matrix_check_with_deny_policy(
            "Secret",
            HttpMethod::Post,
            "/api/query/Secret",
            r#"{"where":{}}"#,
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
    }

    #[test]
    fn matrix_cursor_pagination_respects_read_policy() {
        matrix_check_with_deny_policy(
            "Secret",
            HttpMethod::Get,
            "/api/entities/Secret/cursor?limit=10",
            "",
            Expect::Rejected,
            Expect::Rejected,
            Expect::Rejected,
        );
    }

    /// One-shot audit of every admin-required GET route. Every entry
    /// here is a route where an anonymous, guest, or authed-non-admin
    /// caller MUST receive 401/403. Adding a new admin GET? Add a row.
    /// Removing a route's admin gate? You'll see this test fail in the
    /// PR diff and can confirm the change is intentional.
    ///
    /// This is the forcing function the 2nd security review asked for:
    /// "every `/api/*` GET that doesn't return admin/non-sensitive
    /// data has an auth gate before the handler". Compile-time
    /// enumeration would be nicer but the route list lives in
    /// `route_inner` as a chain of if-blocks; until that's reified
    /// into data, this table is the gate.
    #[test]
    fn matrix_admin_get_routes_audit() {
        let admin_get_routes: &[(&str, &str)] = &[
            ("/api/scheduler", "list scheduled tasks"),
            ("/api/fn", "enumerate registered functions"),
            ("/api/fn/traces", "function execution traces"),
            ("/api/shards", "shard topology + subscriber counts"),
            ("/api/cache/anykey", "raw cache read"),
            ("/api/pubsub/channels", "list pub/sub channels"),
            ("/api/pubsub/history/anychannel", "channel retained history"),
            ("/api/jobs/stats", "job queue stats"),
            ("/api/jobs/dead", "dead-letter queue"),
            ("/api/jobs", "job list with payloads"),
            ("/api/jobs/some-id", "single job detail"),
            ("/api/workflows/definitions", "workflow definitions"),
            ("/api/workflows", "workflow instance list"),
            ("/api/workflows/some-id", "workflow instance detail"),
        ];
        for (url, label) in admin_get_routes {
            matrix_check(
                HttpMethod::Get,
                url,
                "",
                Expect::Rejected,
                Expect::Rejected,
                Expect::Rejected,
            );
            // re-assert with explicit fail message so the audit log
            // pinpoints which row regressed.
            let _ = label;
        }
    }

    // -----------------------------------------------------------------------
    // Wave 8 / Wave 9 wire-level coverage. The matrix_check helper above
    // builds a real RouterContext with stubbed services and calls
    // route() — these tests assert that the new endpoints exist, are
    // gated correctly, and surface the expected error status when the
    // org isn't configured. Catches regressions at the dispatcher level
    // (path matching, method gates, auth check ordering).
    // -----------------------------------------------------------------------

    #[test]
    fn oauth_refresh_endpoint_requires_auth() {
        // Anon: 401 AUTH_REQUIRED.
        // Guest: has a guest user_id, no linked OAuth account → 404
        //   ACCOUNT_NOT_FOUND. The endpoint doesn't reject guests
        //   explicitly because guests literally cannot have linked
        //   accounts; the 404 is the more honest answer.
        // Authed: same → 404 ACCOUNT_NOT_FOUND.
        matrix_check(
            HttpMethod::Post,
            "/api/auth/oauth/refresh/google",
            "",
            Expect::Rejected,
            Expect::Eq(404),
            Expect::Eq(404),
        );
    }

    #[test]
    fn oauth_refresh_rejects_empty_provider_segment() {
        matrix_check(
            HttpMethod::Post,
            "/api/auth/oauth/refresh/",
            "",
            // Anon: hits auth check first → 401.
            Expect::Rejected,
            // Guest + Authed: 400 MISSING_PROVIDER (empty segment).
            Expect::Eq(400),
            Expect::Eq(400),
        );
    }

    #[test]
    fn org_sso_start_returns_404_when_unconfigured() {
        // Public endpoint — no auth check. Org has no SSO config →
        // 404 SSO_NOT_CONFIGURED for everyone.
        matrix_check(
            HttpMethod::Get,
            "/api/auth/orgs/some-org/sso/start?callback=https://app/cb",
            "",
            Expect::Eq(404),
            Expect::Eq(404),
            Expect::Eq(404),
        );
    }

    #[test]
    fn org_sso_callback_returns_403_for_unknown_state() {
        // Public, but state tokens are single-use + org-scoped. An
        // unknown state token must 403 INVALID_SSO_STATE.
        matrix_check(
            HttpMethod::Get,
            "/api/auth/orgs/some-org/sso/callback?state=fake&code=fake",
            "",
            Expect::Eq(403),
            Expect::Eq(403),
            Expect::Eq(403),
        );
    }

    #[test]
    fn saml_start_returns_404_when_unconfigured() {
        matrix_check(
            HttpMethod::Get,
            "/api/auth/orgs/some-org/saml/start?callback=https://app/cb",
            "",
            Expect::Eq(404),
            Expect::Eq(404),
            Expect::Eq(404),
        );
    }

    #[test]
    fn saml_acs_returns_404_when_unconfigured() {
        // Public endpoint. Org with no SAML config → 404
        // SAML_NOT_CONFIGURED before any form-body parse / signature
        // verification. Confirms the dispatcher reaches the handler
        // (vs returning 405 / 404 from path-matching).
        matrix_check(
            HttpMethod::Post,
            "/api/auth/orgs/some-org/saml/acs",
            "",
            Expect::Eq(404),
            Expect::Eq(404),
            Expect::Eq(404),
        );
    }

    #[test]
    fn sso_discover_returns_400_when_email_missing() {
        // Public discovery endpoint. No email param → 400.
        matrix_check(
            HttpMethod::Get,
            "/api/auth/sso/discover",
            "",
            Expect::Eq(400),
            Expect::Eq(400),
            Expect::Eq(400),
        );
    }

    #[test]
    fn sso_discover_returns_404_when_no_org_claims_domain() {
        matrix_check(
            HttpMethod::Get,
            "/api/auth/sso/discover?email=user@nobody-claims-this.com",
            "",
            Expect::Eq(404),
            Expect::Eq(404),
            Expect::Eq(404),
        );
    }

    #[test]
    fn sso_config_crud_requires_org_membership() {
        // Anon → 401 AUTH_REQUIRED. Guest + authed pass the auth gate
        // but fail the membership lookup → 404 ORG_NOT_FOUND. Same
        // pattern as the existing org CRUD endpoints — confirmed by
        // the matching shape in saml_config_crud_requires_org_membership.
        matrix_check(
            HttpMethod::Get,
            "/api/auth/orgs/some-org/sso",
            "",
            Expect::Rejected,
            Expect::Eq(404),
            Expect::Eq(404),
        );
        matrix_check(
            HttpMethod::Put,
            "/api/auth/orgs/some-org/sso",
            r#"{"issuer_url":"https://idp","client_id":"x","client_secret":"y"}"#,
            Expect::Rejected,
            Expect::Eq(404),
            Expect::Eq(404),
        );
        matrix_check(
            HttpMethod::Delete,
            "/api/auth/orgs/some-org/sso",
            "",
            Expect::Rejected,
            Expect::Eq(404),
            Expect::Eq(404),
        );
    }

    #[test]
    fn saml_config_crud_requires_org_membership() {
        matrix_check(
            HttpMethod::Get,
            "/api/auth/orgs/some-org/saml",
            "",
            Expect::Rejected,
            Expect::Eq(404),
            Expect::Eq(404),
        );
        matrix_check(
            HttpMethod::Put,
            "/api/auth/orgs/some-org/saml",
            r#"{"idp_entity_id":"x","idp_sso_url":"https://idp","idp_x509_cert_pem":"y"}"#,
            Expect::Rejected,
            Expect::Eq(404),
            Expect::Eq(404),
        );
        matrix_check(
            HttpMethod::Delete,
            "/api/auth/orgs/some-org/saml",
            "",
            Expect::Rejected,
            Expect::Eq(404),
            Expect::Eq(404),
        );
    }

    // Keep the warning silencer until this is used.
    #[allow(dead_code)]
    const _TOUCH_ATOMIC_BOOL: AtomicBool = AtomicBool::new(false);

    // -----------------------------------------------------------------------
    // Round-3 hardening tests
    // -----------------------------------------------------------------------

    #[test]
    fn redact_email_keeps_two_chars_and_domain() {
        assert_eq!(super::redact_email("alice@acme.com"), "al***@acme.com");
        assert_eq!(super::redact_email("a@b.io"), "a***@b.io");
        assert_eq!(super::redact_email("ab@x.io"), "ab***@x.io");
        // Pathological inputs don't crash; just return a marker.
        assert_eq!(super::redact_email("not-an-email"), "***");
        assert_eq!(super::redact_email(""), "***");
        // Multi-byte chars in local-part don't slice mid-codepoint.
        assert_eq!(super::redact_email("éric@x.io"), "ér***@x.io");
    }

    #[test]
    fn public_manifest_strips_policy_expressions() {
        use pylon_kernel::{AppManifest, ManifestPolicy, MANIFEST_VERSION};
        let m = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "t".into(),
            version: "0.0.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![ManifestPolicy {
                name: "ownerOnly".into(),
                entity: Some("Todo".into()),
                allow_read: Some("auth.userId == data.ownerId".into()),
                allow_update: Some("auth.userId == data.ownerId".into()),
                ..Default::default()
            }],
            auth: Default::default(),
        };
        let pub_m = super::public_manifest(&m);
        let p = &pub_m.policies[0];
        // Name + entity preserved so client tooling can map "denied
        // by ownerOnly" errors to the human label.
        assert_eq!(p.name, "ownerOnly");
        assert_eq!(p.entity.as_deref(), Some("Todo"));
        // Expressions stripped.
        assert_eq!(p.allow, "");
        assert!(p.allow_read.is_none());
        assert!(p.allow_update.is_none());
        // The full manifest still has them — sanity check the test
        // didn't accidentally mutate the input.
        assert_eq!(
            m.policies[0].allow_read.as_deref(),
            Some("auth.userId == data.ownerId")
        );
    }

    /// Regression: every browser-302 auth callback (OAuth GET, OAuth
    /// form_post, org SSO, SAML ACS) must emit BOTH the new
    /// domain-scoped session cookie AND a Max-Age=0 for the host-only
    /// variant of the same name. Without the pairing, any device that
    /// holds a host-only cookie from before PYLON_COOKIE_DOMAIN was set
    /// keeps sending two cookies on every request and the server
    /// arbitrarily picks one — manifested on iOS Chrome OAuth as a
    /// /dashboard bounce back to /login. `set_browser_session_cookie`
    /// is the choke point all four callbacks route through.
    #[test]
    fn browser_session_cookie_pairs_host_only_clear() {
        let cookie_config = CookieConfig {
            name: "test_session".into(),
            domain: Some(".example.com".into()),
            secure: true,
            same_site: pylon_auth::SameSite::Lax,
            max_age_secs: 60,
            path: "/".into(),
        };
        let anon = AuthContext::anonymous();
        with_ctx_cookie(false, &anon, cookie_config, |ctx| {
            ctx.set_browser_session_cookie("test-token-12345");
            let headers = ctx.take_response_headers();
            let set_cookies: Vec<&str> = headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("Set-Cookie"))
                .map(|(_, v)| v.as_str())
                .collect();
            assert_eq!(
                set_cookies.len(),
                2,
                "expected new-cookie + host-only-clear; got {set_cookies:?}"
            );
            let new_cookie = set_cookies
                .iter()
                .find(|v| v.contains("test-token-12345"))
                .expect("new session cookie present");
            assert!(new_cookie.contains("Domain=.example.com"));
            assert!(new_cookie.contains("HttpOnly"));
            let clear_cookie = set_cookies
                .iter()
                .find(|v| !v.contains("test-token-12345"))
                .expect("host-only clear present");
            assert!(
                !clear_cookie.contains("Domain="),
                "host-only clear must omit Domain= so it targets the bare host"
            );
            assert!(clear_cookie.contains("Max-Age=0"));
        });
    }

    /// Sanity-check the no-domain case: without a Domain attribute
    /// there's no host-only variant to clear, so we emit a single
    /// Set-Cookie. (Avoids double-setting in dev where the cookie is
    /// already host-only.)
    #[test]
    fn browser_session_cookie_skips_clear_when_no_domain() {
        let cookie_config = CookieConfig {
            name: "test_session".into(),
            domain: None,
            secure: true,
            same_site: pylon_auth::SameSite::Lax,
            max_age_secs: 60,
            path: "/".into(),
        };
        let anon = AuthContext::anonymous();
        with_ctx_cookie(false, &anon, cookie_config, |ctx| {
            ctx.set_browser_session_cookie("test-token-no-domain");
            let headers = ctx.take_response_headers();
            let set_cookies: Vec<&str> = headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("Set-Cookie"))
                .map(|(_, v)| v.as_str())
                .collect();
            assert_eq!(set_cookies.len(), 1, "no Domain → single Set-Cookie");
            assert!(set_cookies[0].contains("test-token-no-domain"));
        });
    }

    /// Recording ChangeNotifier — captures every notify_session_changed
    /// call so regression tests can assert which user_id + tenant the
    /// router pushed. The change/presence/crdt methods are no-ops
    /// because the tests below only exercise the auth-mutation
    /// surface.
    struct RecordingNotifier {
        session_changes: std::sync::Mutex<Vec<(String, Option<String>)>>,
    }

    impl RecordingNotifier {
        fn new() -> Self {
            Self {
                session_changes: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn session_changes(&self) -> Vec<(String, Option<String>)> {
            self.session_changes.lock().unwrap().clone()
        }
    }

    impl ChangeNotifier for RecordingNotifier {
        fn notify(&self, _event: &pylon_sync::ChangeEvent) {}
        fn notify_presence(&self, _json: &str) {}
        fn notify_session_changed(&self, user_id: &str, tenant_id: Option<&str>) {
            self.session_changes
                .lock()
                .unwrap()
                .push((user_id.to_string(), tenant_id.map(String::from)));
        }
    }

    /// Regression: DELETE /api/auth/session must push session-changed
    /// for the now-revoked user so OTHER tabs of theirs refresh and
    /// surface the signed-out state. Pre-fix, other tabs kept
    /// showing the previous user's rows until the next pull cycle.
    /// (Select-org's push has the same shape; both routes funnel
    /// through `ctx.notifier.notify_session_changed` so one test
    /// covers the wiring of both.)
    #[test]
    fn session_revoke_pushes_session_changed() {
        let notifier = RecordingNotifier::new();
        let user_id = "user-bob";

        let recorder = &notifier;
        let mut captured_token: Option<String> = None;
        with_ctx_notifier(false, &AuthContext::anonymous(), recorder, |ctx| {
            // Mint a real session through SessionStore so the token
            // resolves through the normal bearer path. Capture the
            // token to use as the auth bearer in the next call.
            let session = ctx.session_store.create(user_id.to_string());
            captured_token = Some(session.token);
        });
        let token = captured_token.expect("session minted");

        let auth = AuthContext {
            user_id: Some(user_id.into()),
            is_admin: false,
            is_guest: false,
            roles: vec![],
            tenant_id: None,
            api_key_id: None,
            api_key_scopes: None,
            is_trusted_device: false,
        };
        with_ctx_notifier(false, &auth, recorder, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Delete,
                "/api/auth/session",
                "",
                Some(&token),
            );
            assert_eq!(status, 200);
        });

        let changes = notifier.session_changes();
        assert_eq!(
            changes,
            vec![(user_id.to_string(), None)],
            "session revoke must push session-changed(user, None); got {changes:?}",
        );
    }
}

// ---------------------------------------------------------------------------
// Regression: User-row field projection on EVERY read path.
//
// Apps that need cross-user displayName lookups (chat-style UIs) have to
// register a permissive User read policy. Without field-level redaction
// that would also leak `passwordHash` through the change feed and the
// raw entity API. These tests pin the framework default — strip
// `passwordHash` + `_*` even when the policy says "allow read of every
// row" — across all the read paths that surface User rows.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod user_projection_tests {
    use super::*;
    use pylon_kernel::{ManifestAuthUserConfig, MANIFEST_VERSION};

    fn cfg(expose: &[&str], hide: &[&str]) -> ManifestAuthUserConfig {
        ManifestAuthUserConfig {
            entity: "User".into(),
            expose: expose.iter().map(|s| s.to_string()).collect(),
            hide: hide.iter().map(|s| s.to_string()).collect(),
            admin_field: None,
        }
    }

    fn user_row() -> serde_json::Value {
        serde_json::json!({
            "id": "u-1",
            "email": "alice@example.com",
            "displayName": "Alice",
            "passwordHash": "$argon2id$v=19$super-secret",
            "_internalFlag": true,
            "secretToken": "tok-xyz",
        })
    }

    #[test]
    fn defaults_strip_passwordHash_and_underscore_fields() {
        let projected = maybe_project_user_row("User", user_row(), &cfg(&[], &[]));
        let obj = projected.as_object().unwrap();
        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("email"));
        assert!(obj.contains_key("displayName"));
        assert!(!obj.contains_key("passwordHash"), "passwordHash leaked");
        assert!(!obj.contains_key("_internalFlag"), "_internalFlag leaked");
        // App-specific secrets without an explicit hide stay readable —
        // the framework can't know `secretToken` is sensitive. That's
        // what the manifest's `auth.user.hide` config is for.
        assert!(obj.contains_key("secretToken"));
    }

    #[test]
    fn manifest_hide_strips_app_specific_secrets() {
        let projected = maybe_project_user_row("User", user_row(), &cfg(&[], &["secretToken"]));
        let obj = projected.as_object().unwrap();
        assert!(!obj.contains_key("secretToken"));
        // Defaults still apply on top of the hide list.
        assert!(!obj.contains_key("passwordHash"));
    }

    #[test]
    fn expose_allowlist_drops_everything_else() {
        let projected = maybe_project_user_row("User", user_row(), &cfg(&["displayName"], &[]));
        let obj = projected.as_object().unwrap();
        assert!(obj.contains_key("id"), "id always included");
        assert!(obj.contains_key("displayName"));
        assert!(!obj.contains_key("email"));
        assert!(!obj.contains_key("passwordHash"));
    }

    #[test]
    fn non_user_entity_passes_through_untouched() {
        let row = serde_json::json!({
            "id": "m-1",
            "body": "hi",
            "passwordHash": "looks-sensitive-but-isnt-a-user-row",
        });
        let projected = maybe_project_user_row("Message", row.clone(), &cfg(&[], &[]));
        // The framework only redacts the configured User entity.
        // Other entities flow through verbatim — application code is
        // responsible for keeping non-User secrets out of read schemas.
        assert_eq!(projected, row);
    }

    #[test]
    fn handle_get_strips_passwordHash_from_user_response() {
        struct LeakyUserStore {
            manifest: pylon_kernel::AppManifest,
        }
        impl pylon_http::DataStore for LeakyUserStore {
            fn manifest(&self) -> &pylon_kernel::AppManifest {
                &self.manifest
            }
            fn insert(
                &self,
                _e: &str,
                _d: &serde_json::Value,
            ) -> Result<String, pylon_http::DataError> {
                Ok("u-1".into())
            }
            fn get_by_id(
                &self,
                entity: &str,
                id: &str,
            ) -> Result<Option<serde_json::Value>, pylon_http::DataError> {
                if entity == "User" && id == "u-1" {
                    return Ok(Some(serde_json::json!({
                        "id": "u-1",
                        "email": "alice@example.com",
                        "displayName": "Alice",
                        "passwordHash": "$argon2id$v=19$super-secret",
                    })));
                }
                Ok(None)
            }
            fn list(&self, _e: &str) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
                Ok(vec![])
            }
            fn list_after(
                &self,
                _e: &str,
                _a: Option<&str>,
                _l: usize,
            ) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
                Ok(vec![])
            }
            fn update(
                &self,
                _e: &str,
                _i: &str,
                _d: &serde_json::Value,
            ) -> Result<bool, pylon_http::DataError> {
                Ok(true)
            }
            fn delete(&self, _e: &str, _i: &str) -> Result<bool, pylon_http::DataError> {
                Ok(true)
            }
            fn lookup(
                &self,
                _e: &str,
                _f: &str,
                _v: &str,
            ) -> Result<Option<serde_json::Value>, pylon_http::DataError> {
                Ok(None)
            }
            fn link(
                &self,
                _e: &str,
                _i: &str,
                _r: &str,
                _t: &str,
            ) -> Result<bool, pylon_http::DataError> {
                Ok(true)
            }
            fn unlink(&self, _e: &str, _i: &str, _r: &str) -> Result<bool, pylon_http::DataError> {
                Ok(true)
            }
            fn query_filtered(
                &self,
                _e: &str,
                _f: &serde_json::Value,
            ) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
                Ok(vec![])
            }
            fn query_graph(
                &self,
                _q: &serde_json::Value,
            ) -> Result<serde_json::Value, pylon_http::DataError> {
                Ok(serde_json::json!({}))
            }
            fn aggregate(
                &self,
                _e: &str,
                _s: &serde_json::Value,
            ) -> Result<serde_json::Value, pylon_http::DataError> {
                Ok(serde_json::json!({}))
            }
            fn transact(
                &self,
                _o: &[serde_json::Value],
            ) -> Result<(bool, Vec<serde_json::Value>), pylon_http::DataError> {
                Ok((true, vec![]))
            }
            fn search(
                &self,
                _e: &str,
                _q: &serde_json::Value,
            ) -> Result<serde_json::Value, pylon_http::DataError> {
                Ok(serde_json::json!({}))
            }
        }

        let manifest = pylon_kernel::AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
        };
        let store = LeakyUserStore { manifest };
        let (status, body) = handle_get(&store, "User", "u-1");
        assert_eq!(status, 200);
        assert!(
            !body.contains("passwordHash"),
            "passwordHash leaked through handle_get response: {body}"
        );
        // Confirm the projection is field-strip, not full-redact —
        // displayName must still come through so chat UIs render.
        assert!(body.contains("displayName"));
        assert!(body.contains("Alice"));
    }
}
