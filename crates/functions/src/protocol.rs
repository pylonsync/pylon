//! Bidirectional NDJSON protocol between Rust runtime and TypeScript process.
//!
//! Messages are newline-delimited JSON objects. Each function invocation gets
//! a unique `call_id` for multiplexing concurrent calls over a single connection.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Rust → TypeScript messages
// ---------------------------------------------------------------------------

/// Invoke a function on the TypeScript side.
#[derive(Debug, Clone, Serialize)]
pub struct CallMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str, // always "call"
    pub call_id: String,
    pub fn_name: String,
    pub fn_type: FnType,
    pub args: serde_json::Value,
    pub auth: AuthInfo,
    /// HTTP request context — present only when the action is invoked via
    /// a custom HTTP route (`defineRoute` binding). Actions called from
    /// other actions via `ctx.runAction` or from jobs don't get this.
    /// Enables Stripe-webhook-style signature verification + access to
    /// raw headers/body the router would otherwise discard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestInfo>,
}

/// HTTP request metadata forwarded to TypeScript actions invoked via
/// `defineRoute` bindings. All fields are strings so the TS side can use
/// them directly without re-parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestInfo {
    /// Uppercased method — `POST`, `GET`, etc.
    pub method: String,
    /// Full request path (with query string if any).
    pub path: String,
    /// Lowercased header names → values. Multi-value headers are joined
    /// with `, ` per RFC 7230. This trades some fidelity for a map shape
    /// that's ergonomic to consume from TS.
    pub headers: std::collections::HashMap<String, String>,
    /// The exact bytes of the request body, UTF-8-decoded. Webhook
    /// signature verification (Stripe, GitHub) needs the bytes that were
    /// signed, so this is NOT the parsed JSON.
    pub raw_body: String,
}

impl CallMessage {
    pub fn new(
        call_id: String,
        fn_name: String,
        fn_type: FnType,
        args: serde_json::Value,
        auth: AuthInfo,
    ) -> Self {
        Self {
            msg_type: "call",
            call_id,
            fn_name,
            fn_type,
            args,
            auth,
            request: None,
        }
    }

    /// Attach HTTP request metadata (used when the call originated from a
    /// `defineRoute` HTTP binding rather than a programmatic invocation).
    pub fn with_request(mut self, request: RequestInfo) -> Self {
        self.request = Some(request);
        self
    }
}

/// Render an SSR route on the TypeScript side. Sent from Rust to Bun
/// when an incoming HTTP GET matches a file-based SSR route in the
/// manifest. The Bun-side `@pylonsync/ssr` adapter resolves the
/// component from the `component` path, calls
/// `renderToReadableStream(<App />)`, and streams chunks back via
/// `RenderChunk` messages, terminating with `RenderDone` (or
/// `RenderError` on failure).
///
/// `route_path` is the canonical pattern (e.g. `/blog/:slug`),
/// `url` is the incoming concrete path (`/blog/hello-world`).
/// `params` is the pre-extracted dynamic-segment map. `auth` mirrors
/// the standard call envelope so `<Page>` can render auth-aware UI.
#[derive(Debug, Clone, Serialize)]
pub struct RenderRouteMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str, // always "render_route"
    pub call_id: String,
    pub component: String,
    /// Layout chain walked root → leaf. Each entry is a project-
    /// relative module path. The Bun adapter dynamically imports
    /// each layout's default export and wraps them around the page
    /// component (leaf → root assembly so the outermost layout's
    /// children is the next layout, terminating with the page).
    /// Empty when no layouts apply.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layouts: Vec<String>,
    pub route_path: String,
    pub url: String,
    pub params: serde_json::Value,
    pub search_params: serde_json::Value,
    pub headers: std::collections::HashMap<String, String>,
    pub cookies: std::collections::HashMap<String, String>,
    pub auth: AuthInfo,
    /// Whether the request carried a session cookie (by name), exposed to the
    /// page as the identity-FREE `props.session.exists` so it can render a binary
    /// auth-aware nav without reading real `auth` (which would taint caching).
    /// Presence, not validity — a present-but-invalid cookie reads `true` and the
    /// client resolves the real (anonymous) session. Default false (back-compat).
    #[serde(default)]
    pub session_present: bool,
    /// Initial HTTP status the Bun-side response controller starts at.
    /// `None` (default 200) for normal page renders; `Some(404)` when the
    /// host dispatches a `not-found.tsx` render for an unmatched URL so the
    /// boundary streams at 404 without the component calling `setStatus`.
    /// Skipped on serialize when `None` so existing renders are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_status: Option<u16>,
}

impl RenderRouteMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        call_id: String,
        component: String,
        layouts: Vec<String>,
        route_path: String,
        url: String,
        params: serde_json::Value,
        search_params: serde_json::Value,
        headers: std::collections::HashMap<String, String>,
        cookies: std::collections::HashMap<String, String>,
        auth: AuthInfo,
        session_present: bool,
        initial_status: Option<u16>,
    ) -> Self {
        Self {
            msg_type: "render_route",
            call_id,
            component,
            layouts,
            route_path,
            url,
            params,
            search_params,
            headers,
            cookies,
            auth,
            session_present,
            initial_status,
        }
    }
}

/// Outbound message for a `route.ts` form/method handler (#276). Like
/// `RenderRouteMessage` but carries the HTTP `method` + the parsed `form`
/// fields instead of layouts. The Bun `ssr-form-runtime` picks the matching
/// handler export (POST/PUT/PATCH/DELETE), runs it with the form + request
/// context, and replies through the SAME `response_start` / `render_chunk` /
/// `render_done` (+ `db`) protocol a render uses — except the `db` ops here
/// may WRITE (a form handler is mutation-shaped), so the host answers them
/// against a broadcast-capable store.
#[derive(Debug, Clone, Serialize)]
pub struct HandleFormMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str, // always "handle_form"
    pub call_id: String,
    pub component: String,
    pub route_path: String,
    pub method: String,
    pub url: String,
    pub params: serde_json::Value,
    pub search_params: serde_json::Value,
    /// Parsed form fields: name → value (string) or values (array of strings
    /// for repeated fields). `application/x-www-form-urlencoded` + multipart
    /// TEXT fields.
    pub form: serde_json::Value,
    pub headers: std::collections::HashMap<String, String>,
    pub cookies: std::collections::HashMap<String, String>,
    pub auth: AuthInfo,
}

impl HandleFormMessage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        call_id: String,
        component: String,
        route_path: String,
        method: String,
        url: String,
        params: serde_json::Value,
        search_params: serde_json::Value,
        form: serde_json::Value,
        headers: std::collections::HashMap<String, String>,
        cookies: std::collections::HashMap<String, String>,
        auth: AuthInfo,
    ) -> Self {
        Self {
            msg_type: "handle_form",
            call_id,
            component,
            route_path,
            method,
            url,
            params,
            search_params,
            form,
            headers,
            cookies,
            auth,
        }
    }
}

/// Result of a DB operation, sent back to TypeScript.
///
/// `op_id` is echoed from the incoming `DbOpMessage.op_id` when present.
/// The TS runtime uses it to demux concurrent DB ops inside a single
/// function call (e.g. `Promise.all([ctx.db.get(a), ctx.db.get(b)])`).
/// Absent `op_id` keeps legacy TS runtimes compatible.
#[derive(Debug, Clone, Serialize)]
pub struct DbResultMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str, // always "result"
    pub call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

impl DbResultMessage {
    pub fn ok(call_id: String, data: serde_json::Value) -> Self {
        Self {
            msg_type: "result",
            call_id,
            op_id: None,
            data: Some(data),
            error: None,
        }
    }

    pub fn ok_with_op(call_id: String, op_id: Option<String>, data: serde_json::Value) -> Self {
        Self {
            msg_type: "result",
            call_id,
            op_id,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(call_id: String, code: &str, message: &str) -> Self {
        Self {
            msg_type: "result",
            call_id,
            op_id: None,
            data: None,
            error: Some(ErrorInfo {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }

    pub fn err_with_op(call_id: String, op_id: Option<String>, code: &str, message: &str) -> Self {
        Self {
            msg_type: "result",
            call_id,
            op_id,
            data: None,
            error: Some(ErrorInfo {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// TypeScript → Rust messages
// ---------------------------------------------------------------------------

/// A message from the TypeScript handler back to Rust.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum TsMessage {
    /// DB operation request.
    #[serde(rename = "db")]
    Db(DbOpMessage),

    /// Stream a chunk to the HTTP client (SSE).
    #[serde(rename = "stream")]
    Stream(StreamChunkMessage),

    /// Schedule a function for later execution.
    #[serde(rename = "schedule")]
    Schedule(ScheduleMessage),

    /// Elevate the call's auth context after the handler has done
    /// its own authentication check (HMAC signature verify, JWT
    /// validation, custom token check). Used by webhook receivers
    /// — they're necessarily public (external systems POST to
    /// them) but need to schedule internal:true workers after
    /// they've proven the request came from a trusted source.
    ///
    /// The framework doesn't verify the developer actually checked
    /// anything before elevating — that's on them. The `reason`
    /// field is mandatory so every elevation is auditable.
    #[serde(rename = "elevate_auth")]
    ElevateAuth(ElevateAuthMessage),

    /// Cancel a previously scheduled function.
    #[serde(rename = "cancel_schedule")]
    CancelSchedule(CancelScheduleMessage),

    /// Call another function (for actions calling queries/mutations).
    #[serde(rename = "run_fn")]
    RunFn(RunFnMessage),

    /// Send a transactional email via the runtime's configured provider.
    /// Only valid from action handlers — mutations + queries reject by
    /// the time the dispatcher hands the message off.
    #[serde(rename = "send_email")]
    SendEmail(SendEmailMessage),

    /// Call the configured LLM provider. The request body is the
    /// Anthropic Messages shape (model, messages, system, tools,
    /// max_tokens). Available from all handler types — agents
    /// commonly run tool-use loops out of queries.
    #[serde(rename = "llm_complete")]
    LlmComplete(LlmCompleteMessage),

    /// `ctx.connections.*` op. Wired to the runtime's
    /// ConnectionManager.
    #[serde(rename = "connection")]
    Connection(ConnectionOpMessage),

    /// Function completed successfully.
    #[serde(rename = "return")]
    Return(ReturnMessage),

    /// Function failed with an error.
    #[serde(rename = "error")]
    Error(ErrorMessage),

    /// Initial handshake from the runtime: the list of functions it loaded.
    /// Sent once at startup before any other message.
    #[serde(rename = "ready")]
    Ready(ReadyMessage),

    /// SSR — emit the response status + headers BEFORE the body chunks
    /// start flowing. Fire-and-forget; the host wires it into the HTTP
    /// response head. Sent once per render, before any RenderChunk.
    /// If the handler returns without emitting this, the host defaults
    /// to status 200 + `Content-Type: text/html; charset=utf-8`.
    #[serde(rename = "response_start")]
    ResponseStart(ResponseStartMessage),

    /// SSR — a chunk of the rendered HTML body. Bytes are base64-
    /// encoded so newlines + binary safety work over the NDJSON pipe.
    /// Fire-and-forget; the host writes the decoded bytes to the
    /// streaming response body as they arrive (the existing pipe is
    /// already non-buffered — Bun stdout → Rust mpsc → tiny_http
    /// chunked transfer encoding).
    #[serde(rename = "render_chunk")]
    RenderChunk(RenderChunkMessage),

    /// SSR — the renderer finished cleanly. No more body chunks
    /// coming. The host closes the response body. Carries no payload
    /// beyond `call_id` (status + headers came in ResponseStart).
    #[serde(rename = "render_done")]
    RenderDone(RenderDoneMessage),

    /// Hydration — Bun finished bundling the client entry; the
    /// `path` field carries the absolute path on disk to read from.
    /// One-shot reply to `BundleClientMessage`.
    #[serde(rename = "bundle_client_result")]
    BundleClientResult(BundleClientResultMessage),
}

impl TsMessage {
    /// The `call_id` this message belongs to, or `None` for the one-shot
    /// startup `Ready` handshake (the only message with no call to route to).
    ///
    /// This is what makes concurrent calls over a single Bun connection safe:
    /// the host's reader thread routes EVERY message to the waiting call by
    /// this id, so two in-flight renders can never receive each other's output.
    /// Every variant except `Ready` carries a `call_id` (the protocol was
    /// designed for it); a missing arm here would silently drop that message
    /// type, so this match is deliberately exhaustive (no `_` catch-all).
    pub fn call_id(&self) -> Option<&str> {
        match self {
            TsMessage::Ready(_) => None,
            TsMessage::Db(m) => Some(&m.call_id),
            TsMessage::Stream(m) => Some(&m.call_id),
            TsMessage::Schedule(m) => Some(&m.call_id),
            TsMessage::ElevateAuth(m) => Some(&m.call_id),
            TsMessage::CancelSchedule(m) => Some(&m.call_id),
            TsMessage::RunFn(m) => Some(&m.call_id),
            TsMessage::SendEmail(m) => Some(&m.call_id),
            TsMessage::LlmComplete(m) => Some(&m.call_id),
            TsMessage::Connection(m) => Some(&m.call_id),
            TsMessage::Return(m) => Some(&m.call_id),
            TsMessage::Error(m) => Some(&m.call_id),
            TsMessage::ResponseStart(m) => Some(&m.call_id),
            TsMessage::RenderChunk(m) => Some(&m.call_id),
            TsMessage::RenderDone(m) => Some(&m.call_id),
            TsMessage::BundleClientResult(m) => Some(&m.call_id),
        }
    }
}

/// Handshake payload from the TS runtime.
#[derive(Debug, Clone, Deserialize)]
pub struct ReadyMessage {
    #[serde(default)]
    pub functions: Vec<crate::registry::FnDef>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A database operation request from TypeScript.
#[derive(Debug, Clone, Deserialize)]
pub struct DbOpMessage {
    pub call_id: String,
    /// Optional per-RPC id minted by the TS side. When present, the Rust
    /// reply echoes it back on `DbResultMessage.op_id` so the TS runtime
    /// can demux concurrent DB ops from a single handler (Promise.all).
    /// Legacy TS runtimes that don't send op_id keep the old behavior:
    /// only one in-flight RPC per call_id at a time.
    #[serde(default)]
    pub op_id: Option<String>,
    pub op: DbOp,
    pub entity: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub relation: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
    /// Cursor pagination — `paginate` op only. Opaque id-after cursor.
    #[serde(default)]
    pub after: Option<String>,
    /// Cursor pagination — `paginate` op only. Requested page size.
    #[serde(default)]
    pub limit: Option<u32>,
    /// When `true`, this op was emitted by `ctx.db.unsafe.*` instead
    /// of plain `ctx.db.*`. The framework's caller-aware policy gate
    /// (gated by `PYLON_STRICT_FN_POLICIES=1`, currently in Phase 1)
    /// skips enforcement on `unsafe` ops — the developer has
    /// explicitly asserted that this call needs to bypass row-level
    /// access control (admin tools, cron sweeps, cross-tenant
    /// reads). Plain `ctx.db.*` is the safe default; the unsafe
    /// path requires the keyword + an explicit comment per
    /// codebase convention (`pylon lint` will flag bare
    /// `ctx.db.unsafe.*` without a justification comment in a
    /// future rule).
    ///
    /// Default `false` — old TS runtimes that don't send the field
    /// keep the safe-default shape. Skipped on serialize when
    /// `false` so the wire format stays compact.
    #[serde(default, skip_serializing_if = "is_false_local")]
    pub unsafe_op: bool,
}

// Used via `skip_serializing_if = "is_false_local"` — serde
// resolves the path by string so rustc can't see the use.
#[allow(dead_code)]
fn is_false_local(b: &bool) -> bool {
    !*b
}

/// Database operations available to TypeScript functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DbOp {
    Get,
    List,
    /// Cursor-paginated list. Uses `after` + `limit` on [`DbOpMessage`].
    /// Response shape: `{ page: Row[], nextCursor: string | null, isDone: bool }`.
    Paginate,
    Insert,
    Update,
    Delete,
    Lookup,
    Query,
    QueryGraph,
    Link,
    Unlink,
    /// Faceted full-text search. The query body is whatever the entity
    /// declares in its `search:` config — query string, filters,
    /// facets, sort, page, pageSize. Carried on `data`.
    /// Response shape: `{ hits, facetCounts, total, tookMs }`.
    Search,
    /// Acquire a transaction-scoped advisory lock. Used to close
    /// TOCTOU races on quota / uniqueness checks. The lock key travels
    /// on `entity` (we reuse the field rather than carving a new
    /// protocol slot — the message shape is `{ op: "advisory_lock",
    /// entity: "<key>" }`). Held until the mutation tx commits or
    /// rolls back.
    AdvisoryLock,
}

/// A stream chunk to forward to the HTTP client as SSE.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamChunkMessage {
    pub call_id: String,
    pub data: String,
    /// Optional event type for SSE (defaults to "message").
    #[serde(default)]
    pub event: Option<String>,
}

/// Schedule a function for future execution.
#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleMessage {
    pub call_id: String,
    pub fn_name: String,
    pub args: serde_json::Value,
    /// Run after this many milliseconds.
    #[serde(default)]
    pub delay_ms: Option<u64>,
    /// Run at this Unix timestamp (ms since epoch).
    #[serde(default)]
    pub run_at: Option<u64>,
}

/// Cancel a scheduled function.
#[derive(Debug, Clone, Deserialize)]
pub struct CancelScheduleMessage {
    pub call_id: String,
    pub schedule_id: String,
}

/// Elevate the call's auth context. See `TsMessage::ElevateAuth`.
#[derive(Debug, Clone, Deserialize)]
pub struct ElevateAuthMessage {
    pub call_id: String,
    /// Flip caller_is_admin to true for the rest of this call. Allows
    /// subsequent `ctx.scheduler.runAfter` calls to enqueue internal:
    /// true targets without bouncing through an HTTP loopback with the
    /// platform admin token.
    #[serde(default)]
    pub admin: bool,
    /// Human-readable rationale for the elevation. Logged at INFO so
    /// an operator can audit who elevated and why (e.g. "github
    /// webhook hmac verified"). Mandatory — empty reason is rejected
    /// at the handler so accidental elevations always carry blame.
    pub reason: String,
}

/// Call another function from within an action.
#[derive(Debug, Clone, Deserialize)]
pub struct RunFnMessage {
    pub call_id: String,
    pub fn_name: String,
    pub fn_type: FnType,
    pub args: serde_json::Value,
}

/// Send a transactional email via the runtime's configured provider.
/// Mirror of Pylon's auth-side EmailAdapter — exposed to actions so
/// app code (invites, notifications, password handoffs) can use the
/// same transport without rebuilding HTTP clients per provider.
#[derive(Debug, Clone, Deserialize)]
pub struct SendEmailMessage {
    pub call_id: String,
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// Call the configured LLM provider. The `request` field is forwarded
/// to [`pylon_runtime::llm::LlmClient::complete`] verbatim — the TS
/// side builds it once, the host doesn't transform the shape (so
/// callers can use new fields like `tool_choice` without a host
/// release).
#[derive(Debug, Clone, Deserialize)]
pub struct LlmCompleteMessage {
    pub call_id: String,
    /// Anthropic Messages-shaped request body. See pylon_runtime::llm
    /// for the canonical Rust type; the TS SDK normalizes provider
    /// differences.
    pub request: serde_json::Value,
}

/// `ctx.connections.<op>(name, ...)` request. `op` is one of:
/// - `"authorize_url"` — returns `{url}`. Optional
///   `post_redirect` in body for post-callback browser destination.
/// - `"get"` — returns `{access_token, scope, expires_at}`.
///   Refreshes silently when needed.
/// - `"list"` — returns `{connections: [...]}` for the signed-in user.
/// - `"disconnect"` — removes the stored `_Connection` row.
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionOpMessage {
    pub call_id: String,
    pub op: String,
    /// Op-specific payload. For all ops except `list`, must include
    /// `name`. `authorize_url` accepts optional `post_redirect`.
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// Function returned successfully.
#[derive(Debug, Clone, Deserialize)]
pub struct ReturnMessage {
    pub call_id: String,
    pub value: serde_json::Value,
}

/// SSR — initial response headers emitted before body chunks.
#[derive(Debug, Clone, Deserialize)]
pub struct ResponseStartMessage {
    pub call_id: String,
    /// HTTP status code (200, 404, 500, ...). Default 200 if absent.
    #[serde(default)]
    pub status: Option<u16>,
    /// Response headers. Multi-value headers join with `, `.
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
}

/// SSR — a base64-encoded chunk of the rendered response body.
#[derive(Debug, Clone, Deserialize)]
pub struct RenderChunkMessage {
    pub call_id: String,
    /// Base64-encoded body bytes. Decoded by the host before writing
    /// to the streaming response.
    pub data: String,
}

/// SSR — render completed cleanly. No more chunks.
#[derive(Debug, Clone, Deserialize)]
pub struct RenderDoneMessage {
    pub call_id: String,
}

/// Hydration — host asks Bun to bundle the client entry. Bun
/// discovers `app/**/page.tsx` + `app/**/layout.tsx`, generates a
/// hydration entry that imports each, wraps it with React +
/// react-dom/client, calls Bun.build({ target: "browser" }), and
/// writes the result to `.pylon/client.js` under the project cwd.
/// Returns the absolute path so the host can stream it directly
/// from disk on `/_pylon/client.js` requests — no base64 round-trip
/// over NDJSON for a ~150kB bundle.
///
/// Phase 1.5d: built once at boot, cached forever. File-watcher
/// invalidation comes with the dev-time HMR plumbing in Phase 1.5e.
#[derive(Debug, Clone, Serialize)]
pub struct BundleClientMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str, // always "bundle_client"
    pub call_id: String,
    /// Project-relative directory holding the route tree
    /// (`<app_dir>/**​/page.tsx`). Usually `"app"`; the full-stack app
    /// that namespaces its frontend under a subdir (e.g. `web/app` via
    /// `discoverAppRoutes({appDir:"web/app"})`) sends that here so the
    /// client bundler walks the same dir the SSR manifest was built
    /// from. Empty → the Bun side defaults to `"app"`.
    pub app_dir: String,
}

impl BundleClientMessage {
    pub fn new(call_id: String, app_dir: String) -> Self {
        Self {
            msg_type: "bundle_client",
            call_id,
            app_dir,
        }
    }
}

/// Hydration — Bun → host after `Bun.build` finishes. Uses per-route
/// entries with shared chunks, returning:
///   - `path`: absolute path to the manifest JSON
///     (`<cwd>/.pylon/client-build/manifest.json`). The host reads
///     this to discover which entry file + chunks each route needs.
///   - `outdir`: absolute path to the build output directory. The
///     host serves any file under it at `/_pylon/build/<rel>`.
///
/// `path` stayed in the schema (instead of being renamed
/// `manifest_path`) so single-version old/new Bun runtimes can talk
/// to a matching-version host — the wire field name is the contract.
#[derive(Debug, Clone, Deserialize)]
pub struct BundleClientResultMessage {
    pub call_id: String,
    /// Absolute path to the manifest JSON. Empty on error.
    #[serde(default)]
    pub path: String,
    /// Absolute path to the build output directory. Files under
    /// this directory are served at `/_pylon/build/<relative>`.
    /// Empty on error.
    #[serde(default)]
    pub outdir: String,
    /// Optional failure message. Mutually exclusive with usable
    /// `path` / `outdir`.
    #[serde(default)]
    pub error: Option<String>,
}

/// Function failed.
#[derive(Debug, Clone, Deserialize)]
pub struct ErrorMessage {
    pub call_id: String,
    pub code: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// Function type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FnType {
    Query,
    Mutation,
    Action,
}

/// Auth context passed to function handlers.
///
/// Mirrors the runtime's `AuthContext` fields a mutation can
/// legitimately read: authenticated user id, admin flag, active
/// tenant, and JWT/manifest-issued roles. Functions that gate on
/// `ctx.auth.tenantId` (anything org-scoped) or `ctx.auth.roles`
/// (RBAC-style policies) need these forwarded — reactive query
/// re-runs in particular have to carry the FULL identity captured
/// at subscribe time, not a stripped-down subset, or role-gated
/// policies see empty roles on re-run and silently deny.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub is_admin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
}

/// Error info in protocol messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_message_serializes() {
        let msg = CallMessage::new(
            "c1".into(),
            "placeBid".into(),
            FnType::Mutation,
            serde_json::json!({"lotId": "lot_1", "amount": 100}),
            AuthInfo {
                user_id: Some("user_1".into()),
                is_admin: false,
                tenant_id: None,
                roles: Vec::new(),
            },
        );
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"call\""));
        assert!(json.contains("\"fn_type\":\"mutation\""));
    }

    #[test]
    fn ts_message_deserializes_db_op() {
        let json = r#"{"type":"db","call_id":"c1","op":"get","entity":"Lot","id":"lot_1"}"#;
        let msg: TsMessage = serde_json::from_str(json).unwrap();
        match msg {
            TsMessage::Db(db) => {
                assert_eq!(db.call_id, "c1");
                assert_eq!(db.op, DbOp::Get);
                assert_eq!(db.entity, "Lot");
                assert_eq!(db.id.as_deref(), Some("lot_1"));
            }
            _ => panic!("expected Db message"),
        }
    }

    #[test]
    fn ts_message_deserializes_stream() {
        let json = r#"{"type":"stream","call_id":"c1","data":"hello"}"#;
        let msg: TsMessage = serde_json::from_str(json).unwrap();
        match msg {
            TsMessage::Stream(s) => {
                assert_eq!(s.data, "hello");
                assert!(s.event.is_none());
            }
            _ => panic!("expected Stream message"),
        }
    }

    #[test]
    fn ts_message_deserializes_return() {
        let json = r#"{"type":"return","call_id":"c1","value":{"ok":true}}"#;
        let msg: TsMessage = serde_json::from_str(json).unwrap();
        match msg {
            TsMessage::Return(r) => {
                assert_eq!(r.value, serde_json::json!({"ok": true}));
            }
            _ => panic!("expected Return message"),
        }
    }

    #[test]
    fn ts_message_deserializes_schedule() {
        let json = r#"{"type":"schedule","call_id":"c1","fn_name":"closeLot","args":{"lotId":"x"},"delay_ms":5000}"#;
        let msg: TsMessage = serde_json::from_str(json).unwrap();
        match msg {
            TsMessage::Schedule(s) => {
                assert_eq!(s.fn_name, "closeLot");
                assert_eq!(s.delay_ms, Some(5000));
            }
            _ => panic!("expected Schedule message"),
        }
    }

    #[test]
    fn db_result_ok() {
        let msg = DbResultMessage::ok("c1".into(), serde_json::json!({"id": "x"}));
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn db_result_err() {
        let msg = DbResultMessage::err("c1".into(), "NOT_FOUND", "not found");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"error\""));
        assert!(!json.contains("\"data\""));
    }
}
