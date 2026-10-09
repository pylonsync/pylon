//! Function runner — executes TypeScript functions via the bidirectional protocol.
//!
//! The runner manages the connection to the Bun/Deno process and mediates
//! all communication. It handles DB operations, stream forwarding, scheduling,
//! and transaction management.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Per-runner demux table: `call_id` → the channel feeding that call's recv
/// loop. The single reader thread routes EVERY inbound message to the right
/// call by id (see [`TsMessage::call_id`]), which is what lets multiple calls
/// (renders, functions) run concurrently over one Bun connection — they can
/// never receive each other's messages. Registered/unregistered per call via
/// [`CallRoute`] (RAII).
type RouteTable = Mutex<HashMap<String, Sender<TsMessage>>>;

/// RAII registration for one call's demux route — removes the `call_id` from
/// the table (and decrements the in-flight gauge) when the call's frame drops,
/// whether it returned, errored, or unwound on a panic.
struct CallRoute {
    table: Arc<RouteTable>,
    gauge: Arc<AtomicUsize>,
    call_id: String,
}
impl Drop for CallRoute {
    fn drop(&mut self) {
        if let Ok(mut g) = self.table.lock() {
            g.remove(&self.call_id);
        }
        self.gauge.fetch_sub(1, Ordering::Relaxed);
    }
}

use pylon_http::DataStore;

use crate::protocol::*;
use crate::trace::{TraceBuilder, TraceLog};

/// Default ceiling on how long a single function call may go without
/// producing a frame (an IDLE timeout — activity restarts it). Holds the
/// SQLite write lock for mutations, so this is also a backstop against a
/// runaway TS handler blocking the whole DB. Override via
/// [`FnRunner::set_call_timeout`] or `PYLON_FN_CALL_TIMEOUT` (server-side).
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Runaway backstop: however chatty a call is, it may not outlive
/// `timeout × this`. With the idle-timeout semantics a handler emitting a
/// stream chunk every few seconds could otherwise run forever; 10× turns
/// "forever" into "ten budgets", and a genuinely long agent run declares
/// `timeout: 600` on its def for a 100-minute ceiling.
pub const MAX_CALL_LIFETIME_MULTIPLIER: u32 = 10;

/// Clone the call's auth context while applying its current mutable admin
/// state. In-call elevation changes only `is_admin`; identity, tenancy, and
/// roles remain anchored to the entry auth snapshot.
fn current_auth_snapshot(auth: &AuthInfo, is_admin: bool) -> AuthInfo {
    AuthInfo {
        is_admin,
        ..auth.clone()
    }
}

// ---------------------------------------------------------------------------
// Stream callback — receives SSE chunks during execution
// ---------------------------------------------------------------------------

/// One `ctx.stream` chunk as the handler emitted it. `event` carries
/// the SSE event type from `ctx.stream.writeEvent(event, data)`; plain
/// `ctx.stream.write(data)` leaves it `None`.
#[derive(Debug, Clone, Copy)]
pub struct StreamChunk<'a> {
    pub data: &'a str,
    pub event: Option<&'a str>,
}

/// Callback invoked for each stream chunk during function execution.
/// The server layer converts these into SSE events on the HTTP response
/// (framing `event:` when present — pre-widening, `writeEvent`'s event
/// type was silently dropped here).
pub type StreamCallback = Box<dyn FnMut(StreamChunk<'_>) + Send>;

/// Byte-flavored stream callback for SSR. Receives base64-decoded
/// chunks of the rendered response body. Kept separate from
/// `StreamCallback` (string SSE) so the existing SSE path stays
/// untouched — that path framed every chunk as `data: <text>\n\n`,
/// SSR streams raw HTML bytes.
pub type ByteStreamCallback = Box<dyn FnMut(&[u8]) + Send>;

/// Callback invoked when the SSR handler emits `response_start`.
/// Receives the HTTP status + header map BEFORE any body chunks.
/// The host wires these into the response head. Fires at most once
/// per render.
pub type ResponseStartCallback =
    Box<dyn FnMut(u16, std::collections::HashMap<String, String>) + Send>;

/// Information about the function CALLING `ctx.scheduler.runAfter/runAt`,
/// passed to the schedule hook so it can enforce "public functions can't
/// smuggle work to internal:true targets via the scheduler" — caught in
/// the 2026-05-10 codex pass-3 audit (P1). Without this gate any public
/// action accepting a fn_name argument becomes an internal-fn proxy, and
/// the dispatched job runs with anonymous auth.
#[derive(Debug, Clone)]
pub struct ScheduleCallerInfo {
    /// Whether the calling function declares `internal: true`. Internal
    /// callers are allowed to schedule any target (including other
    /// internal:true cron self-perpetuation patterns).
    pub caller_internal: bool,
    /// Whether the calling function ran with an admin AuthContext.
    /// Admins skip the gate entirely.
    pub caller_is_admin: bool,
    /// User id of the scheduling caller, if any. Propagated to the
    /// job so the scheduled callback runs with the caller's identity
    /// (matches the semantics of a direct call).
    pub caller_user_id: Option<String>,
    /// Active tenant of the scheduling caller, if any. Same propagation
    /// rationale as `caller_user_id`.
    pub caller_tenant_id: Option<String>,
    /// Whether the scheduling caller was an anonymous guest session.
    /// Propagated so the job runs as a guest too — dropping it would let
    /// a guest schedule work that executes with the standing of a real
    /// account.
    pub caller_is_guest: bool,
}

/// Callback invoked when a function calls `ctx.scheduler.runAfter/runAt`.
/// Returns `Ok(job_id)` on success or `Err(msg)` on persistence/queue
/// failure (or on the internal-target gate refusing the enqueue). The
/// runner reports the error back to the calling handler so users don't
/// get a silent `{scheduled: true, id: ""}`.
pub type ScheduleHook = Box<
    dyn Fn(
            &str,
            serde_json::Value,
            Option<u64>,
            Option<u64>,
            ScheduleCallerInfo,
        ) -> Result<String, String>
        + Send
        + Sync,
>;

/// Callback invoked when a function calls `ctx.scheduler.cancel(id)`.
/// Returns `Ok(true)` when a pending or buffered job was cancelled,
/// `Ok(false)` when the job was unknown, running, or finished, and `Err`
/// when the cancel could not be persisted (the job still runs).
///
/// `store` is the call's store: inside a Postgres mutation it is the held
/// transaction, so the cancel commits or rolls back with the mutation.
pub type CancelScheduleHook =
    Box<dyn Fn(&str, &dyn DataStore) -> Result<bool, String> + Send + Sync>;

/// Callback invoked when a running function asks to run *another* function
/// (action → query/mutation). The wrapper is responsible for any per-type
/// setup — notably wrapping mutations in their own BEGIN/COMMIT, which
/// can't happen inside `call_inner` because that path is called with the
/// outer action's non-transactional store.
///
/// Returns the nested function's return value or a `FnCallError`-shaped
/// `(code, message)` pair. The runner translates the error back into the
/// NDJSON protocol reply so the TS side sees the same shape it always did.
///
/// The last argument is `Some` only for `ctx.agents.run`: the nested call
/// is then an action that must be started with
/// [`FnRunner::call_agent_inner`], so the child knows it is an agent run.
pub type NestedCallHookFn = dyn Fn(
        &str,
        FnType,
        serde_json::Value,
        AuthInfo,
        Option<crate::protocol::AgentInvocation>,
    ) -> Result<serde_json::Value, (String, String)>
    + Send
    + Sync;
pub type NestedCallHook = Box<NestedCallHookFn>;

/// Callback for `ctx.files.signedUrl(fileId, {ttlSecs})`. Takes the file id
/// and an optional TTL, returns the signed download path (or an error pair).
/// Installed by the runtime, which owns the signing secret; without it,
/// `ctx.files.signedUrl` returns FILES_SIGNING_NOT_CONFIGURED.
pub type FileUrlSigner =
    Box<dyn Fn(&str, Option<u64>) -> Result<String, (String, String)> + Send + Sync>;

/// Callback for `ctx.files.store` / `ctx.files.delete`. Takes the op and the
/// calling user's auth; returns the JSON result or an error pair. Installed
/// by the runtime, which owns the storage backend.
pub type FilesOpHookFn = dyn Fn(&crate::protocol::FilesOp, &AuthInfo) -> Result<serde_json::Value, (String, String)>
    + Send
    + Sync;
pub type FilesOpHook = Box<FilesOpHookFn>;

/// Callback for `ctx.shards.ticket(...)`. Takes the request and the calling
/// user's id, returns the signed ticket (or an error pair). Installed by the
/// runtime, which owns the ticket secret.
pub type ShardTicketSigner = Box<
    dyn Fn(
            &crate::protocol::SignShardTicketMessage,
            Option<&str>,
        ) -> Result<String, (String, String)>
        + Send
        + Sync,
>;

/// Callback for `ctx.shards.create/stop/get/list`. Installed by the runtime
/// when the app declares WebAssembly shards.
pub type ShardOpHook = Box<ShardOpHookFn>;

pub type ShardOpHookFn = dyn Fn(&crate::protocol::ShardOpMessage) -> Result<serde_json::Value, (String, String)>
    + Send
    + Sync;

/// Callback invoked when an action calls `ctx.email.send(to, subject, body)`.
/// Returns Ok(()) on transport success, Err(reason) on failure.
///
/// The runner forwards this to the runtime's EmailAdapter (which knows
/// about PYLON_EMAIL_PROVIDER + credentials). Without this hook installed,
/// `ctx.email.send` returns a "transport not configured" error instead
/// of silently no-op'ing — apps shouldn't think email sent when it didn't.
pub type EmailHookFn = dyn Fn(&pylon_kernel::EmailMessage) -> Result<(), String> + Send + Sync;
pub type EmailHook = Box<EmailHookFn>;

/// Hard cap on the total base64 attachment payload of one email (bytes of
/// base64 text, ≈ 11MB of raw file data after the 4/3 inflation). Enforced
/// on both sides of the pipe — the TS runtime rejects before framing, and
/// this guard catches anything that slips through — because an unbounded
/// attachment rides a single NDJSON line into an unbounded allocation.
pub const EMAIL_MAX_ATTACHMENT_B64_BYTES: usize = 15 * 1024 * 1024;
/// Cap on attachment COUNT per email, matched in the TS runtime.
pub const EMAIL_MAX_ATTACHMENTS: usize = 20;

/// Callback invoked when a function calls `ctx.llm.complete({...})`.
///
/// The `request` is the same JSON shape as `/api/llm/complete` accepts
/// (Anthropic Messages: messages, system, tools, model, max_tokens,
/// temperature). The hook is responsible for the actual provider call
/// + model allowlist check + usage logging.
///
/// Returns the full Anthropic-style response body (model + content +
/// stop_reason + usage) on success; Err with a code + message that
/// surfaces to the TS handler as a thrown error from `ctx.llm.complete`.
pub type LlmHookFn = dyn Fn(&serde_json::Value, &AuthInfo) -> Result<serde_json::Value, (String, String)>
    + Send
    + Sync;
pub type LlmHook = Box<LlmHookFn>;

/// Callback for `ctx.llm.stream(...)`. Same contract as [`LlmHook`],
/// plus an `on_event` sink the host calls for each provider event as
/// it arrives and a `cancel` flag. The hook blocks until the stream
/// completes and returns the assembled final response.
///
/// The runner calls the hook on its own thread, so the call's read loop
/// keeps serving the handler's frames (`ctx.stream.write`, db ops) while
/// the provider generates. `cancel` becomes true when the handler aborts
/// the stream (`llm_cancel`), or when the call ends or times out while
/// the stream is still running; the hook must then
/// stop reading the provider response and drop it, which closes the
/// HTTP connection so the provider stops generating.
///
/// `on_event` receives the serialized `StreamEvent`. It must not be
/// retained past the call.
pub type LlmStreamHookFn = dyn Fn(
        &serde_json::Value,
        &AuthInfo,
        &mut dyn FnMut(serde_json::Value),
        &AtomicBool,
    ) -> Result<serde_json::Value, (String, String)>
    + Send
    + Sync;
pub type LlmStreamHook = Box<LlmStreamHookFn>;

/// Callback for `ctx.llm.embed(texts)`. Same contract as [`LlmHook`]
/// but routed to the embeddings provider (a separate config axis:
/// PYLON_EMBEDDINGS_PROVIDER openai|voyage).
pub type LlmEmbedHookFn = dyn Fn(&serde_json::Value, &AuthInfo) -> Result<serde_json::Value, (String, String)>
    + Send
    + Sync;
pub type LlmEmbedHook = Box<LlmEmbedHookFn>;

/// Callback for `ctx.rooms.broadcast(room, topic, data)`. Returns
/// whether the event reached a live room — `false` means the room had
/// no members, which is informational, not an error.
pub type RoomBroadcastHook =
    Box<dyn Fn(&str, &str, serde_json::Value) -> Result<bool, (String, String)> + Send + Sync>;

/// Callback for `ctx.audit.*`. Receives the call's current auth (after
/// any in-call elevation) so the host stamps the real actor and tenant.
pub type AuditOpHook = Box<
    dyn Fn(
            &crate::protocol::AuditOpMessage,
            &AuthInfo,
        ) -> Result<serde_json::Value, (String, String)>
        + Send
        + Sync,
>;

/// Callback for `ctx.workflows.*` (start / send_event). Wired to the
/// runtime's WorkflowEngine; returns the op's JSON result.
pub type WorkflowOpHook = Box<
    dyn Fn(&crate::protocol::WorkflowOpMessage) -> Result<serde_json::Value, (String, String)>
        + Send
        + Sync,
>;

/// Callback for `ctx.connections.{authorizeUrl,get,disconnect}`.
///
/// Args: op name (`"authorize_url"` | `"get"` | `"disconnect"` |
/// `"list"`), the typed request payload (connection name +
/// optional fields), and the caller's auth context.
///
/// Returns the JSON body to ship back to the TS handler on
/// success; `Err((code, message))` propagates as a typed
/// throwable with `err.code`.
pub type ConnectionHookFn = dyn Fn(&str, &serde_json::Value, &AuthInfo) -> Result<serde_json::Value, (String, String)>
    + Send
    + Sync;
pub type ConnectionHook = Box<ConnectionHookFn>;

// ---------------------------------------------------------------------------
// Function runner
// ---------------------------------------------------------------------------

/// Decision interface for the caller-aware policy gate that
/// runs on every `ctx.db.*` op inside a function handler. Set
/// on a runner via [`FnRunner::set_policy_gate`]; consulted by
/// [`execute_db_op`] when `PYLON_STRICT_FN_POLICIES=1` is set
/// and the op didn't carry `unsafe_op: true`.
///
/// Decoupled from `pylon_policy` so this crate doesn't have to
/// take that dep (which would form a cycle through router →
/// functions → policy → router). Runtime supplies a small
/// adapter that calls into `pylon_policy::PolicyEngine`.
/// Keeps read-local policy state alive until the database operation completes.
pub trait PolicyReadScope {}

pub trait PolicyGate: Send + Sync {
    /// A read scope must not keep authorization answers across database operations.
    fn begin_read(&self) -> Option<Box<dyn PolicyReadScope>> {
        None
    }

    /// Decide whether `op` on `entity` is allowed for the
    /// caller described by `auth`.
    ///
    /// - `existing` is the stored row for Update and Delete (None when
    ///   the row doesn't exist, and for Read and Insert).
    /// - `data` is the new row for Insert and the patch for Update
    ///   or the returned row for Read (None for Delete).
    ///
    /// Returns `Ok(())` to allow, or `Err((code, reason))` to deny.
    /// The runner surfaces the denial as a `DataError` with the
    /// supplied code; the TS handler sees it as a regular
    /// thrown error from `ctx.db.*`.
    fn check_op(
        &self,
        op: PolicyOp,
        entity: &str,
        auth: &crate::protocol::AuthInfo,
        existing: Option<&serde_json::Value>,
        data: Option<&serde_json::Value>,
    ) -> Result<(), (String, String)>;

    /// Check access before a scan. Row rules are checked on the results.
    fn check_read_scan(&self, entity: &str, auth: &AuthInfo) -> Result<(), (String, String)> {
        self.check_op(PolicyOp::Read, entity, auth, None, None)
    }

    /// Reject searches whose aggregates can include forbidden rows.
    fn check_read_aggregate(&self, entity: &str, auth: &AuthInfo) -> Result<(), (String, String)> {
        self.check_op(PolicyOp::Read, entity, auth, None, None)
    }

    /// Post-process the rows returned by a CLIENT-VISIBLE read (SSR
    /// `serverData.*`, whose results are serialized into the browser-visible
    /// `__PYLON_DATA__` hydration blob) so they get the SAME treatment as the
    /// entity/sync read API: drop rows the caller can't read per the entity's
    /// read policy,
    /// and strip `server_only` / `passwordHash` fields before they cross to the
    /// client. Server-function `ctx.db.*` reads never call this (server-trust);
    /// neither does `serverData.unsafe.*`.
    ///
    /// Default impl returns rows unchanged — stub gates and non-runtime
    /// backends don't filter. The runtime adapter implements the real fence.
    fn filter_client_read(
        &self,
        _entity: &str,
        _auth: &crate::protocol::AuthInfo,
        rows: Vec<serde_json::Value>,
    ) -> Vec<serde_json::Value> {
        rows
    }

    /// Fence a CLIENT-VISIBLE faceted-search result (SSR `serverData.search`),
    /// mirroring the entity search route: reject a row-DEPENDENT read policy
    /// (faceted aggregates would leak counts for rows the caller can't read),
    /// then per-hit filter + project. `result` is the `{ hits, facetCounts,
    /// total, tookMs }` envelope. Returns `Err((code, message))` to deny.
    ///
    /// Default impl returns the result unchanged — stub gates don't filter.
    fn filter_client_search(
        &self,
        _entity: &str,
        _auth: &crate::protocol::AuthInfo,
        result: serde_json::Value,
    ) -> Result<serde_json::Value, (String, String)> {
        Ok(result)
    }

    /// Fence a CLIENT-VISIBLE vector-search result (SSR
    /// `serverData.vectorSearch`). Same aggregate-safety reasoning as
    /// `filter_client_search`: top-k similarity ranks over EVERY row, so a
    /// row-dependent read policy is rejected; then per-hit filter + project
    /// the `doc` of each `{ id, score, doc }` hit.
    ///
    /// Default impl returns the result unchanged — stub gates don't filter.
    fn filter_client_vector_search(
        &self,
        _entity: &str,
        _auth: &crate::protocol::AuthInfo,
        result: serde_json::Value,
    ) -> Result<serde_json::Value, (String, String)> {
        Ok(result)
    }
}

/// Coarse action classification for the policy gate. Mirrors
/// `pylon_policy::EntityAction` but lives here so this crate
/// stays free of that dep — see [`PolicyGate`] rationale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyOp {
    Read,
    Insert,
    Update,
    Delete,
}

/// Manages the TypeScript process and executes function calls.
pub struct FnRunner {
    process: Mutex<Option<Child>>,
    /// Stdin half — guarded so concurrent senders don't interleave bytes.
    /// Shared (`Arc`) with the threads that run `ctx.llm.stream` provider
    /// calls, which write `llm_event` frames and their terminal reply
    /// while the call's read loop keeps running.
    stdin: Arc<Mutex<Option<std::process::ChildStdin>>>,
    /// Per-call demux table for the CURRENT child. The reader thread routes
    /// each inbound message to the waiting call by `call_id`, so concurrent
    /// calls (renders, functions) multiplex over the one Bun connection
    /// instead of serializing. `None` until `start()`; replaced on respawn (a
    /// fresh table per child so a dead child's reader can't touch the new one).
    routes: Mutex<Option<Arc<RouteTable>>>,
    /// Number of calls currently in flight on this runner. Shared with each
    /// call's `CallRoute` guard. Feeds the health probe: an idle runner
    /// (in_flight == 0) is healthy; a busy one is healthy only while messages
    /// keep flowing (`last_msg_at`).
    in_flight: Arc<AtomicUsize>,
    /// Epoch-millis of the last message the reader received from the child.
    /// Shared with the reader thread. With multiplexing there's no held lock to
    /// probe for "wedged", so the health signal is "are messages still flowing
    /// while calls are in flight?".
    last_msg_at: Arc<AtomicU64>,
    call_counter: AtomicU64,
    pub trace_log: TraceLog,
    schedule_hook: Mutex<Option<ScheduleHook>>,
    cancel_schedule_hook: Mutex<Option<CancelScheduleHook>>,
    /// Optional override for nested function calls (action → query/mutation).
    /// When set, the runner delegates `RunFn` messages to this hook so the
    /// caller can wrap mutations in their own transaction. When absent, we
    /// fall back to the old recursive path (no transaction for nested
    /// mutations — documented limitation).
    /// An `Arc` so a nested call runs without holding this lock: a nested
    /// call can nest again (an agent started by `ctx.agents.run` calls
    /// `ctx.runMutation`), and concurrent calls must not serialize here.
    nested_call_hook: Mutex<Option<Arc<NestedCallHookFn>>>,
    file_url_signer: Mutex<Option<FileUrlSigner>>,
    files_op_hook: Mutex<Option<Arc<FilesOpHookFn>>>,
    shard_ticket_signer: Mutex<Option<ShardTicketSigner>>,
    /// An `Arc` so a call runs after the lock is released: a hook that
    /// panics must not poison it for every later `ctx.shards` call.
    shard_op_hook: Mutex<Option<std::sync::Arc<ShardOpHookFn>>>,
    /// Optional handler for `ctx.email.send(...)`. Apps that don't configure
    /// an email transport see `ctx.email.send` reject with an explicit
    /// error so silently-dropped invite emails surface in the action's
    /// error response.
    email_hook: Mutex<Option<Arc<EmailHookFn>>>,
    /// Optional handler for `ctx.llm.complete(...)`. When unset, the
    /// hook returns an explicit "LLM_NOT_CONFIGURED" error so authors
    /// see the gap instead of getting a silent no-op.
    llm_hook: Mutex<Option<Arc<LlmHookFn>>>,
    /// Optional handler for `ctx.llm.stream(...)`. Unset behaves like
    /// `llm_hook` — an explicit LLM_NOT_CONFIGURED error. An `Arc` so a
    /// stream runs on its own thread without holding this lock (holding
    /// it serialized every concurrent stream on the runner).
    llm_stream_hook: Mutex<Option<Arc<LlmStreamHookFn>>>,
    /// Optional handler for `ctx.llm.embed(...)`. Unset returns an
    /// explicit EMBEDDINGS_NOT_CONFIGURED error.
    llm_embed_hook: Mutex<Option<Arc<LlmEmbedHookFn>>>,
    /// Hook for `ctx.rooms.broadcast(...)`. Wires server-originated
    /// room events to the runtime's RoomManager + presence notifier.
    room_broadcast_hook: Mutex<Option<RoomBroadcastHook>>,
    workflow_op_hook: Mutex<Option<WorkflowOpHook>>,
    audit_op_hook: Mutex<Option<AuditOpHook>>,
    /// Hook for `ctx.connections.*`. Wires authorize-url / get /
    /// list / disconnect calls to the runtime's ConnectionManager.
    connection_hook: Mutex<Option<Arc<ConnectionHookFn>>>,
    /// Timeout for `recv()` between protocol messages. A handler that doesn't
    /// reply within this window is treated as stuck.
    call_timeout: Mutex<Duration>,
    /// The command and args that started the runtime. Stored so the supervisor
    /// can respawn on crash without the caller re-passing them.
    started_with: Mutex<Option<(String, Vec<String>)>>,
    /// Caller-aware policy gate. Consulted on every `ctx.db.*` op
    /// when set + `PYLON_STRICT_FN_POLICIES=1` + the op isn't
    /// `unsafe_op`. None when the runtime hasn't wired it (older
    /// embeddings / wasm). See [`PolicyGate`] for the contract.
    policy_gate: Mutex<Option<std::sync::Arc<dyn PolicyGate>>>,
    /// Per-function call deadline overrides (seconds), keyed by function name,
    /// from each function's `timeout` option. Repopulated from the handshake
    /// FnDefs on every start/respawn. A call to a function listed here uses its
    /// timeout instead of the global `call_timeout`; the supervisor reads the
    /// max (via [`max_fn_timeout_secs`]) so its wedge backstop never pre-empts
    /// the longest legitimate call.
    per_fn_timeouts: Mutex<std::collections::HashMap<String, u64>>,
    /// Workflows the TS runtime declared in its ready handshake (from the
    /// app's `workflows/` dir). Repopulated on every start/respawn; the
    /// host registers them with the WorkflowEngine.
    workflows: Mutex<Vec<crate::protocol::WorkflowInfo>>,
    /// When the runner last went from idle to busy (ms epoch). Paired with
    /// `last_msg_at` in the health probe: silence is measured from
    /// max(last frame from the child, start of the current busy period),
    /// so an idle stretch before a call never counts as wedge silence.
    busy_since: Arc<AtomicU64>,
}

impl FnRunner {
    /// Create a new runner with the given trace log capacity.
    pub fn new(trace_capacity: usize) -> Self {
        Self {
            process: Mutex::new(None),
            stdin: Arc::new(Mutex::new(None)),
            routes: Mutex::new(None),
            in_flight: Arc::new(AtomicUsize::new(0)),
            last_msg_at: Arc::new(AtomicU64::new(0)),
            call_counter: AtomicU64::new(0),
            trace_log: TraceLog::new(trace_capacity),
            schedule_hook: Mutex::new(None),
            cancel_schedule_hook: Mutex::new(None),
            nested_call_hook: Mutex::new(None),
            file_url_signer: Mutex::new(None),
            files_op_hook: Mutex::new(None),
            shard_ticket_signer: Mutex::new(None),
            shard_op_hook: Mutex::new(None),
            email_hook: Mutex::new(None),
            llm_hook: Mutex::new(None),
            llm_stream_hook: Mutex::new(None),
            llm_embed_hook: Mutex::new(None),
            room_broadcast_hook: Mutex::new(None),
            workflow_op_hook: Mutex::new(None),
            audit_op_hook: Mutex::new(None),
            connection_hook: Mutex::new(None),
            call_timeout: Mutex::new(DEFAULT_CALL_TIMEOUT),
            started_with: Mutex::new(None),
            policy_gate: Mutex::new(None),
            per_fn_timeouts: Mutex::new(std::collections::HashMap::new()),
            workflows: Mutex::new(Vec::new()),
            busy_since: Arc::new(AtomicU64::new(0)),
        }
    }

    /// True when the child has emitted ANY frame since `start_ms`. The
    /// per-call wedge test: a call that times out while this is false was
    /// running on a child that produced nothing the whole time — an
    /// event-loop wedge (busy sync loop), not a slow await. That child
    /// can't even read a `cancel` frame; killing it (supervisor respawns)
    /// is the only recovery, and doing it here restores the old code's
    /// property that a wedge is cleared within one call timeout.
    fn child_emitted_since(&self, start_ms: u64) -> bool {
        self.last_msg_at.load(Ordering::Relaxed) >= start_ms
    }

    /// Workflows declared by the live TS runtime's ready handshake.
    pub fn workflow_infos(&self) -> Vec<crate::protocol::WorkflowInfo> {
        self.workflows.lock().unwrap().clone()
    }

    /// Record per-function timeout overrides from the handshake FnDefs. Called
    /// after every successful start/respawn so the map tracks the live worker's
    /// functions (a redeploy can change timeouts).
    pub(crate) fn record_fn_timeouts(&self, defs: &[crate::registry::FnDef]) {
        let mut map = self.per_fn_timeouts.lock().unwrap();
        map.clear();
        for d in defs {
            if let Some(secs) = d.timeout_secs {
                if secs > 0 {
                    map.insert(d.name.clone(), secs);
                }
            }
        }
    }

    /// The call deadline for `fn_name`: its per-function `timeout` override if
    /// set, else the global `call_timeout`.
    fn deadline_for(&self, fn_name: &str) -> Duration {
        if let Some(&secs) = self.per_fn_timeouts.lock().unwrap().get(fn_name) {
            return Duration::from_secs(secs);
        }
        *self.call_timeout.lock().unwrap()
    }

    /// The largest per-function timeout (seconds) any registered function
    /// declares, or 0 if none. The supervisor sizes its wedge backstop to
    /// `max(global call timeout, this)` so a busy-but-progressing worker
    /// running a long call isn't respawned out from under it.
    pub fn max_fn_timeout_secs(&self) -> u64 {
        self.per_fn_timeouts
            .lock()
            .unwrap()
            .iter()
            // Framework-internal defs (__pylon_workflow_run declares 600s)
            // must not size the wedge window for the whole runner — an app
            // that merely HAS a workflows/ dir would otherwise stretch the
            // supervisor's kill threshold ~18x for every call. A wedged
            // workflow slice is caught by the per-call zero-frames kill.
            .filter(|(name, _)| !name.starts_with("__pylon_"))
            .map(|(_, secs)| *secs)
            .max()
            .unwrap_or(0)
    }

    /// Install the caller-aware policy gate. The runtime crate
    /// supplies the adapter to `pylon_policy::PolicyEngine`; this
    /// crate stays free of that dep so the trait + indirection
    /// is the public seam.
    ///
    /// Function database operations use this gate when
    /// `PYLON_STRICT_FN_POLICIES=1`. Safe SSR reads always use it.
    pub fn set_policy_gate(&self, gate: std::sync::Arc<dyn PolicyGate>) {
        *self.policy_gate.lock().unwrap() = Some(gate);
    }

    /// Override the per-call timeout. The default is 30s.
    pub fn set_call_timeout(&self, timeout: Duration) {
        *self.call_timeout.lock().unwrap() = timeout;
    }

    /// Install a callback to handle `ctx.scheduler` requests from functions.
    pub fn set_schedule_hook(&self, hook: ScheduleHook) {
        *self.schedule_hook.lock().unwrap() = Some(hook);
    }

    /// Install a callback to handle `ctx.scheduler.cancel` requests.
    pub fn set_cancel_schedule_hook(&self, hook: CancelScheduleHook) {
        *self.cancel_schedule_hook.lock().unwrap() = Some(hook);
    }

    /// Install a callback used for nested function calls (action → query or
    /// mutation). The callback is responsible for transactional wrapping when
    /// the nested fn is a mutation. Without this hook, nested mutations share
    /// the outer action's non-transactional store and writes aren't atomic.
    pub fn set_nested_call_hook(&self, hook: NestedCallHook) {
        *self.nested_call_hook.lock().unwrap() = Some(Arc::from(hook));
    }

    /// Install the signed-file-URL minter backing `ctx.files.signedUrl`.
    /// The runtime installs this with a closure over its signing secret.
    pub fn set_shard_ticket_signer(&self, hook: ShardTicketSigner) {
        *self.shard_ticket_signer.lock().unwrap() = Some(hook);
    }

    /// Install the hook backing `ctx.shards.create/stop/get/list`.
    pub fn set_shard_op_hook(&self, hook: ShardOpHook) {
        *self.shard_op_hook.lock().unwrap() = Some(std::sync::Arc::from(hook));
    }

    pub fn set_files_op_hook(&self, hook: FilesOpHook) {
        *self.files_op_hook.lock().unwrap() = Some(Arc::from(hook));
    }

    pub fn set_file_url_signer(&self, hook: FileUrlSigner) {
        *self.file_url_signer.lock().unwrap() = Some(hook);
    }

    /// Install a callback for `ctx.email.send(to, subject, body)` from
    /// action handlers. Wires through the runtime's configured EmailAdapter.
    /// When unset, `ctx.email.send` returns an explicit
    /// "EMAIL_TRANSPORT_NOT_CONFIGURED" error so authors see the gap
    /// instead of getting a silent no-op.
    pub fn set_email_hook(&self, hook: EmailHook) {
        *self.email_hook.lock().unwrap() = Some(Arc::from(hook));
    }

    /// Install a callback for `ctx.llm.complete(...)` from any handler
    /// (query, mutation, action — queries are intentionally allowed so
    /// agent-tool functions can be defined as queries). The hook reaches
    /// the provider via the LlmClient + applies model-allowlist gating.
    /// When unset, `ctx.llm.complete` rejects with `LLM_NOT_CONFIGURED`.
    pub fn set_llm_hook(&self, hook: LlmHook) {
        *self.llm_hook.lock().unwrap() = Some(Arc::from(hook));
    }

    /// Install a callback for `ctx.llm.stream(...)`. Same gating as
    /// [`Self::set_llm_hook`]; the hook additionally pumps provider
    /// events back to the handler as they arrive.
    pub fn set_llm_stream_hook(&self, hook: LlmStreamHook) {
        *self.llm_stream_hook.lock().unwrap() = Some(Arc::from(hook));
    }

    /// Install a callback for `ctx.llm.embed(texts)`. When unset, the
    /// call rejects with `EMBEDDINGS_NOT_CONFIGURED`.
    pub fn set_llm_embed_hook(&self, hook: LlmEmbedHook) {
        *self.llm_embed_hook.lock().unwrap() = Some(Arc::from(hook));
    }

    /// Install a callback for `ctx.rooms.broadcast(room, topic, data)`.
    /// When unset, the call rejects with `ROOMS_NOT_CONFIGURED`.
    pub fn set_room_broadcast_hook(&self, hook: RoomBroadcastHook) {
        *self.room_broadcast_hook.lock().unwrap() = Some(hook);
    }

    /// Install the `ctx.workflows.*` hook. Without it, calls reject
    /// with `WORKFLOWS_NOT_CONFIGURED`.
    pub fn set_workflow_op_hook(&self, hook: WorkflowOpHook) {
        *self.workflow_op_hook.lock().unwrap() = Some(hook);
    }

    /// Install the `ctx.audit.*` handler.
    pub fn set_audit_op_hook(&self, hook: AuditOpHook) {
        *self.audit_op_hook.lock().unwrap() = Some(hook);
    }

    /// Install the `ctx.connections.*` hook. Without it, calls
    /// reject with `CONNECTIONS_NOT_CONFIGURED`.
    pub fn set_connection_hook(&self, hook: ConnectionHook) {
        *self.connection_hook.lock().unwrap() = Some(Arc::from(hook));
    }

    /// Start the TypeScript process and complete the startup handshake.
    ///
    /// Spawns the child + reader thread, waits for the runtime's `Ready`
    /// message, and only then publishes stdin/inbox/process so callers can
    /// see the runner. This avoids the race where a concurrent `call()`
    /// would consume the `Ready` message and desync the protocol.
    ///
    /// On any failure (spawn, missing pipes, bad handshake, runtime-reported
    /// error) the child is killed before returning so a half-alive process
    /// doesn't survive — important for the supervisor, which uses
    /// `is_alive()` and would otherwise see "still running" forever.
    ///
    /// Returns the function definitions reported by the runtime.
    pub fn start(
        &self,
        command: &str,
        args: &[&str],
    ) -> Result<Vec<crate::registry::FnDef>, String> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("Failed to start function runner: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| kill_and_msg(&mut child, "Failed to capture stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| kill_and_msg(&mut child, "Failed to capture stdout".to_string()))?;

        // A fresh demux table for THIS child. The reader routes call messages
        // here by call_id; the one-shot `Ready` (no call_id) goes to ready_tx.
        let routes: Arc<RouteTable> = Arc::new(Mutex::new(HashMap::new()));
        let (ready_tx, ready_rx): (Sender<TsMessage>, Receiver<TsMessage>) = mpsc::channel();
        let routes_for_reader = Arc::clone(&routes);
        let last_msg_for_reader = Arc::clone(&self.last_msg_at);
        std::thread::Builder::new()
            .name("pylon-fn-reader".into())
            .spawn(move || {
                reader_loop(
                    BufReader::new(stdout),
                    routes_for_reader,
                    ready_tx,
                    last_msg_for_reader,
                )
            })
            .map_err(|e| kill_and_msg(&mut child, format!("Failed to spawn reader thread: {e}")))?;

        // Read Ready BEFORE publishing the routes. The reader sends the
        // call_id-less Ready to its own channel, so there's no risk a
        // concurrent caller's route eats it.
        let ready_timeout = ready_timeout_from_env();
        let ready_msg = match ready_rx.recv_timeout(ready_timeout) {
            Ok(m) => m,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "handshake timeout: TS runtime did not send Ready within {}s \
                     (PYLON_FN_READY_TIMEOUT_SECS raises it)",
                    ready_timeout.as_secs()
                ));
            }
        };
        let defs = match ready_msg {
            TsMessage::Ready(r) => {
                if let Some(err) = r.error {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("Runtime startup error: {err}"));
                }
                // Workflows declared in the app's workflows/ dir ride the
                // same handshake; the host registers them with the
                // WorkflowEngine after spawn (see try_spawn_functions).
                *self.workflows.lock().unwrap() = r.workflows;
                r.functions
            }
            other => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("expected Ready handshake, got {other:?}"));
            }
        };

        // Handshake succeeded — publish.
        *self.stdin.lock().unwrap() = Some(stdin);
        *self.routes.lock().unwrap() = Some(routes);
        *self.process.lock().unwrap() = Some(child);
        *self.started_with.lock().unwrap() = Some((
            command.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        ));
        // Record per-function timeout overrides from this handshake so calls +
        // the supervisor honor them. `respawn()` routes through `start()`, so a
        // redeploy that changes a timeout is picked up here too.
        self.record_fn_timeouts(&defs);

        Ok(defs)
    }

    /// Check if the TypeScript process is running.
    pub fn is_running(&self) -> bool {
        self.process.lock().unwrap().is_some()
    }

    /// Returns true if the child process is alive. Distinct from `is_running`
    /// which only checks that we ever started one — supervisor uses this.
    pub fn is_alive(&self) -> bool {
        let mut guard = self.process.lock().unwrap();
        match guard.as_mut() {
            None => false,
            Some(child) => match child.try_wait() {
                Ok(Some(status)) => {
                    // Say HOW it died (exit code vs signal): stderr is
                    // inherited, so a clean exit(0) prints nothing, and the
                    // supervisor's respawn otherwise hides that the child is
                    // exiting at all.
                    tracing::warn!("[functions] runner child exited: {status:?}");
                    false
                }
                Ok(None) => true, // still running
                Err(_) => false,  // can't tell — assume dead
            },
        }
    }

    /// Deeper "is the runtime responsive?" probe — distinct from
    /// `is_alive` which only checks the OS process. This tries to
    /// derive responsiveness from the MESSAGE FLOW (not a held lock — with
    /// multiplexing no lock is held during a call). An idle runner (no calls in
    /// flight) is healthy by definition. A busy runner is healthy only if the
    /// reader has received SOME message within `timeout`: a runtime that's
    /// genuinely making progress streams render chunks / db round-trips / a
    /// return, so silence-while-busy is the "wedged" signal.
    ///
    /// Used by /health/deep so Fly's health check fails when the bun runtime is
    /// thrashing, even though the HTTP listener itself is still up and answering
    /// /health 200. This is the failure mode that caused the runtime-kill cycle
    /// during a past incident: /health stayed green while every function call
    /// took 30s + got killed, taking all functions offline during respawn.
    ///
    /// Returns Ok(()) when the runtime is responsive within timeout,
    /// Err(reason) when it isn't.
    pub fn health_probe(&self, timeout: Duration) -> Result<(), String> {
        if !self.is_alive() {
            return Err("runtime process not alive".into());
        }
        // No calls in flight → nothing could be wedged → healthy.
        if self.in_flight.load(Ordering::Relaxed) == 0 {
            return Ok(());
        }
        // Busy: healthy only while messages keep arriving. Silence is
        // measured from max(last child frame, start of the current busy
        // period) — see busy_since.
        let last = self
            .last_msg_at
            .load(Ordering::Relaxed)
            .max(self.busy_since.load(Ordering::Relaxed));
        let now = now_millis();
        let silent_for = now.saturating_sub(last);
        if silent_for <= timeout.as_millis() as u64 {
            Ok(())
        } else {
            Err(format!(
                "no runtime message for {silent_for}ms with {} call(s) in flight — wedged",
                self.in_flight.load(Ordering::Relaxed)
            ))
        }
    }

    /// Restart the underlying process using the command/args from the original
    /// `start()` call. The supervisor uses this; callers should not need it.
    /// Returns the freshly-handshaked function definitions. On any failure
    /// the new child has already been killed by `start()`.
    pub fn respawn(&self) -> Result<Vec<crate::registry::FnDef>, String> {
        let started = self
            .started_with
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "Cannot respawn: runner was never started".to_string())?;
        // Drop the dead child + IO before spawning a new one.
        self.kill();
        let arg_refs: Vec<&str> = started.1.iter().map(|s| s.as_str()).collect();
        self.start(&started.0, &arg_refs)
    }

    /// Forcefully kill the child process. Used by the supervisor on timeout
    /// or when the runtime is shutting down. The reader thread will exit
    /// cleanly when its stdout closes.
    pub fn kill(&self) {
        if let Some(mut child) = self.process.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // Drop stdin so the reader thread sees EOF and exits.
        *self.stdin.lock().unwrap() = None;
        // Disconnect every in-flight call NOW (clear the route senders) so each
        // recv returns Disconnected → RUNNER_EXITED immediately, rather than
        // waiting for the reader to observe EOF. Then drop our handle to the
        // table so no new call registers on a dead child.
        if let Some(table) = self.routes.lock().unwrap().take() {
            if let Ok(mut g) = table.lock() {
                g.clear();
            }
        }
    }

    /// Backwards-compatible: `start()` now performs the handshake itself
    /// and returns the function definitions. `handshake()` is a no-op shim
    /// that returns whatever the runtime is currently registered to.
    /// Kept so existing callers (`try_spawn_functions`) compile without churn.
    pub fn handshake(&self) -> Result<Vec<crate::registry::FnDef>, String> {
        Err("handshake is now performed inside start(); use the return value".to_string())
    }

    /// Execute a function call against the TypeScript process.
    ///
    /// For mutations: the caller must hold the write lock and pass a transaction-capable store.
    /// For queries: uses the read pool, no locking required.
    /// For actions: no direct DB access, calls run_fn for nested queries/mutations.
    ///
    /// Returns `(return_value, trace)`. Stream chunks are delivered via the callback.
    pub fn call(
        &self,
        store: &dyn DataStore,
        fn_name: &str,
        fn_type: FnType,
        args: serde_json::Value,
        auth: AuthInfo,
        on_stream: Option<StreamCallback>,
        request: Option<crate::protocol::RequestInfo>,
        stream_id: Option<String>,
    ) -> Result<(serde_json::Value, crate::trace::FnTrace), FnCallError> {
        // Top-level calls MULTIPLEX over the one Bun connection (NDJSON is
        // demuxed by call_id), so there's no serializing lock. Each call —
        // including nested ones (action → query) — registers its own demux
        // route inside `call_inner`.
        self.call_inner_with_caller_internal(
            store, fn_name, fn_type, args, auth, on_stream, request, stream_id, false,
        )
    }

    /// Render an SSR route — peer to `call()` but uses the
    /// `render_route` message + `RenderChunk`/`RenderDone`/`RenderError`
    /// reply protocol instead of `call`/`return`. Streams base64-
    /// decoded body chunks via `on_chunk` and the response head via
    /// `on_response_start`. Returns when the renderer emits
    /// `RenderDone` (Ok) or `Error` (Err).
    ///
    /// `params` holds dynamic-segment matches (e.g. `{slug: "hello"}`).
    /// `search_params` is the parsed query string. `headers` and
    /// `cookies` are forwarded so the page component can render
    /// auth-aware UI. `auth` is the full Pylon auth context, same
    /// shape as a `call()` envelope.
    ///
    /// If `on_response_start` is never invoked before the first
    /// chunk arrives, the host defaults to `200 OK` +
    /// `Content-Type: text/html; charset=utf-8`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_route(
        &self,
        component: &str,
        layouts: Vec<String>,
        route_path: &str,
        url: &str,
        host: &str,
        params: serde_json::Value,
        search_params: serde_json::Value,
        headers: std::collections::HashMap<String, String>,
        cookies: std::collections::HashMap<String, String>,
        auth: crate::protocol::AuthInfo,
        session_present: bool,
        initial_status: Option<u16>,
        design: bool,
        store: &dyn DataStore,
        on_response_start: Option<ResponseStartCallback>,
        on_chunk: ByteStreamCallback,
    ) -> Result<(), FnCallError> {
        self.render_route_inner(
            component,
            layouts,
            route_path,
            url,
            host,
            params,
            search_params,
            headers,
            cookies,
            auth,
            session_present,
            initial_status,
            design,
            store,
            on_response_start,
            on_chunk,
        )
    }

    /// Body of `render_route()` — registers a demux route, then streams.
    #[allow(clippy::too_many_arguments)]
    fn render_route_inner(
        &self,
        component: &str,
        layouts: Vec<String>,
        route_path: &str,
        url: &str,
        host: &str,
        params: serde_json::Value,
        search_params: serde_json::Value,
        headers: std::collections::HashMap<String, String>,
        cookies: std::collections::HashMap<String, String>,
        auth: crate::protocol::AuthInfo,
        session_present: bool,
        initial_status: Option<u16>,
        design: bool,
        store: &dyn DataStore,
        mut on_response_start: Option<ResponseStartCallback>,
        mut on_chunk: ByteStreamCallback,
    ) -> Result<(), FnCallError> {
        use base64::Engine;
        let timeout = *self.call_timeout.lock().unwrap();
        // Idle timeout, same semantics as the call loop: streamed chunks and
        // serverData round-trips restart the budget, the hard deadline caps a
        // chatty runaway.
        let mut deadline = Instant::now() + timeout;
        let hard_deadline = Instant::now() + timeout.saturating_mul(MAX_CALL_LIFETIME_MULTIPLIER);
        let started_ms = now_millis();
        let call_id = format!("r_{}", self.call_counter.fetch_add(1, Ordering::Relaxed));
        // Register this render's demux route BEFORE sending so the reader can
        // route every reply (response_start, chunks, db round-trips, done) to
        // THIS render's channel. `_route` unregisters on scope exit.
        let (_route, rx) = self.register_call(&call_id)?;

        // The page can reach the DB mid-render via the `serverData` handle
        // (React 19 `use()` + Suspense). Those arrive as `{type:"db",...}`
        // frames on this same pipe and are answered by the Db arm below.
        // Auth is captured once (the request's auth) and re-checked per op
        // by the policy gate — reads only, never writes (a GET render must
        // not mutate). Snapshot the gate + strict flag once, same as the
        // call loop (runner.rs call_inner).
        let ssr_auth = auth.clone();
        let policy_gate_snapshot = self.policy_gate.lock().unwrap().clone();
        let strict_policies =
            std::env::var("PYLON_STRICT_FN_POLICIES").ok().as_deref() == Some("1");

        let msg = crate::protocol::RenderRouteMessage::new(
            call_id.clone(),
            component.to_string(),
            layouts,
            route_path.to_string(),
            url.to_string(),
            host.to_string(),
            params,
            search_params,
            headers,
            cookies,
            auth,
            session_present,
            initial_status,
            design,
        );
        self.send(&msg)?;

        loop {
            // Cancel this render if it wedges — the caller then serves
            // stale-on-error from the ISR cache. The child stays alive for
            // its co-tenants; a render that blocked the whole event loop
            // (the homepage outage) is the supervisor's wedge-strike kill.
            let m = self.recv_or_cancel(
                &rx,
                deadline.min(hard_deadline),
                &call_id,
                started_ms,
                "SSR render",
            )?;
            deadline = Instant::now() + timeout;
            match m {
                TsMessage::ResponseStart(rs) if rs.call_id == call_id => {
                    if let Some(ref mut cb) = on_response_start {
                        cb(rs.status.unwrap_or(200), rs.headers);
                    }
                }
                TsMessage::Db(db_msg) if db_msg.call_id == call_id => {
                    // serverData read during render. READ-ONLY: a page render
                    // is a GET; allowing inserts/updates/deletes here would be
                    // a CSRF-shaped hole. Reject anything that isn't a pure
                    // read op (also rejects QueryGraph/Link/Unlink/AdvisoryLock,
                    // which the policy gate can't check). The policy gate +
                    // tenant plugins applied to `store` give the same read
                    // posture as a query function's `ctx.db`.
                    let reply = if policy_op_for(db_msg.op) == Some(PolicyOp::Read) {
                        let (result, _) = execute_db_op(
                            store,
                            &db_msg,
                            policy_gate_snapshot.as_deref(),
                            &ssr_auth,
                            strict_policies,
                        );
                        match result {
                            Ok(data) => DbResultMessage::ok_with_op(
                                call_id.clone(),
                                db_msg.op_id.clone(),
                                data,
                            ),
                            Err(e) => DbResultMessage::err_with_op(
                                call_id.clone(),
                                db_msg.op_id.clone(),
                                &e.code,
                                &e.message,
                            ),
                        }
                    } else {
                        DbResultMessage::err_with_op(
                            call_id.clone(),
                            db_msg.op_id.clone(),
                            "SSR_WRITE_FORBIDDEN",
                            "writes are not allowed during server-side render; \
                             call a mutation/action from a client event handler instead",
                        )
                    };
                    self.send(&reply)?;
                }
                TsMessage::RenderChunk(c) if c.call_id == call_id => {
                    let bytes = match base64::engine::general_purpose::STANDARD.decode(&c.data) {
                        Ok(b) => b,
                        Err(e) => {
                            return Err(FnCallError {
                                code: "RENDER_CHUNK_DECODE_FAILED".into(),
                                message: format!("base64 decode failed: {e}"),
                            });
                        }
                    };
                    on_chunk(&bytes);
                }
                TsMessage::RunFn(run) if run.call_id == call_id => {
                    // `serverData.fn(name, args)` — the page runs a registered
                    // QUERY function with its OWN request auth (anonymous on a
                    // public page), so a gated query can be the single source
                    // of truth for both SSR and client data. Query-only: a GET
                    // render must not mutate (same posture as the
                    // SSR_WRITE_FORBIDDEN arm above). Executed through the
                    // same nested-call hook `ctx.runQuery` uses, so tracing
                    // and store wiring match a function-initiated call.
                    let reply = if run.fn_type != crate::protocol::FnType::Query {
                        DbResultMessage::err(
                            call_id.clone(),
                            "SSR_FN_NOT_QUERY",
                            "only query functions can be called during server-side render",
                        )
                    } else {
                        let hook_result: Option<Result<serde_json::Value, (String, String)>> = {
                            let hook = self.nested_call_hook.lock().unwrap().clone();
                            hook.map(|cb| {
                                cb(
                                    &run.fn_name,
                                    run.fn_type,
                                    run.args.clone(),
                                    ssr_auth.clone(),
                                    None,
                                )
                            })
                        };
                        match hook_result {
                            Some(Ok(value)) => DbResultMessage::ok(call_id.clone(), value),
                            Some(Err((code, msg))) => {
                                DbResultMessage::err(call_id.clone(), &code, &msg)
                            }
                            None => {
                                // No hook installed — direct recursion, same as
                                // the nested-call fallback in the call loop.
                                match self.call_inner(
                                    store,
                                    &run.fn_name,
                                    run.fn_type,
                                    run.args,
                                    ssr_auth.clone(),
                                    None,
                                    None,
                                ) {
                                    Ok((value, _nested_trace)) => {
                                        DbResultMessage::ok(call_id.clone(), value)
                                    }
                                    Err(e) => DbResultMessage::err(
                                        call_id.clone(),
                                        "FN_CALL_FAILED",
                                        &e.message,
                                    ),
                                }
                            }
                        }
                    };
                    self.send(&reply)?;
                }
                TsMessage::RenderDone(d) if d.call_id == call_id => {
                    return Ok(());
                }
                TsMessage::Error(err) if err.call_id == call_id => {
                    return Err(FnCallError {
                        code: err.code,
                        message: err.message,
                    });
                }
                // Different call_id or unrelated message — skip.
                _ => {}
            }
        }
    }

    /// Run a `route.ts` form/method handler — peer to `render_route`, but the
    /// `db` ops it answers may WRITE (a form handler is mutation-shaped). The
    /// caller passes a broadcast-capable `store` so inserts/updates/deletes
    /// fire change events + hooks like a mutation. The handler's ctx exposes
    /// `db` (read+write) + `response` only — it can't emit runFn/schedule
    /// frames, so this loop only needs Db + the response protocol.
    #[allow(clippy::too_many_arguments)]
    pub fn handle_form(
        &self,
        component: &str,
        route_path: &str,
        method: &str,
        url: &str,
        params: serde_json::Value,
        search_params: serde_json::Value,
        form: serde_json::Value,
        body: String,
        headers: std::collections::HashMap<String, String>,
        cookies: std::collections::HashMap<String, String>,
        auth: crate::protocol::AuthInfo,
        store: &dyn DataStore,
        on_response_start: Option<ResponseStartCallback>,
        on_chunk: ByteStreamCallback,
    ) -> Result<(), FnCallError> {
        self.handle_form_inner(
            component,
            route_path,
            method,
            url,
            params,
            search_params,
            form,
            body,
            headers,
            cookies,
            auth,
            store,
            on_response_start,
            on_chunk,
        )
    }

    /// Body of `handle_form()` — registers a demux route, then streams.
    #[allow(clippy::too_many_arguments)]
    fn handle_form_inner(
        &self,
        component: &str,
        route_path: &str,
        method: &str,
        url: &str,
        params: serde_json::Value,
        search_params: serde_json::Value,
        form: serde_json::Value,
        body: String,
        headers: std::collections::HashMap<String, String>,
        cookies: std::collections::HashMap<String, String>,
        auth: crate::protocol::AuthInfo,
        store: &dyn DataStore,
        mut on_response_start: Option<ResponseStartCallback>,
        mut on_chunk: ByteStreamCallback,
    ) -> Result<(), FnCallError> {
        use base64::Engine;
        let timeout = *self.call_timeout.lock().unwrap();
        let mut deadline = Instant::now() + timeout;
        let hard_deadline = Instant::now() + timeout.saturating_mul(MAX_CALL_LIFETIME_MULTIPLIER);
        let started_ms = now_millis();
        let call_id = format!("f_{}", self.call_counter.fetch_add(1, Ordering::Relaxed));
        // Register the demux route before send (same as render) so concurrent
        // form handlers can't receive each other's messages.
        let (_route, rx) = self.register_call(&call_id)?;

        let form_auth = auth.clone();
        let policy_gate_snapshot = self.policy_gate.lock().unwrap().clone();
        let strict_policies =
            std::env::var("PYLON_STRICT_FN_POLICIES").ok().as_deref() == Some("1");

        let msg = crate::protocol::HandleFormMessage::new(
            call_id.clone(),
            component.to_string(),
            route_path.to_string(),
            method.to_string(),
            url.to_string(),
            params,
            search_params,
            form,
            body,
            headers,
            cookies,
            auth,
        );
        self.send(&msg)?;

        loop {
            // Cancel a wedged form handler (same rationale as the SSR render
            // loop); the child stays alive for its co-tenants.
            let m = self.recv_or_cancel(
                &rx,
                deadline.min(hard_deadline),
                &call_id,
                started_ms,
                "form handler",
            )?;
            deadline = Instant::now() + timeout;
            match m {
                TsMessage::ResponseStart(rs) if rs.call_id == call_id => {
                    if let Some(ref mut cb) = on_response_start {
                        cb(rs.status.unwrap_or(200), rs.headers);
                    }
                }
                TsMessage::Db(db_msg) if db_msg.call_id == call_id => {
                    // A form handler is mutation-shaped: reads AND writes are
                    // allowed (the store broadcasts writes + runs hooks). Same
                    // caller-aware policy gate as a mutation's ctx.db in strict
                    // mode; the standard function trust model otherwise (the
                    // handler enforces req.auth itself).
                    let (result, _) = execute_db_op(
                        store,
                        &db_msg,
                        policy_gate_snapshot.as_deref(),
                        &form_auth,
                        strict_policies,
                    );
                    let reply = match result {
                        Ok(data) => {
                            DbResultMessage::ok_with_op(call_id.clone(), db_msg.op_id.clone(), data)
                        }
                        Err(e) => DbResultMessage::err_with_op(
                            call_id.clone(),
                            db_msg.op_id.clone(),
                            &e.code,
                            &e.message,
                        ),
                    };
                    self.send(&reply)?;
                }
                TsMessage::RenderChunk(c) if c.call_id == call_id => {
                    let bytes = match base64::engine::general_purpose::STANDARD.decode(&c.data) {
                        Ok(b) => b,
                        Err(e) => {
                            return Err(FnCallError {
                                code: "RENDER_CHUNK_DECODE_FAILED".into(),
                                message: format!("base64 decode failed: {e}"),
                            });
                        }
                    };
                    on_chunk(&bytes);
                }
                TsMessage::RenderDone(d) if d.call_id == call_id => {
                    return Ok(());
                }
                TsMessage::Error(err) if err.call_id == call_id => {
                    return Err(FnCallError {
                        code: err.code,
                        message: err.message,
                    });
                }
                // Different call_id or unrelated message — skip.
                _ => {}
            }
        }
    }

    /// Result of a `bundle_client` RPC. Phase 1.5e shipped per-route
    /// entries + shared chunks, so the host needs both the manifest
    /// path (for SSR-side script-tag emission) and the output
    /// directory (for serving files at `/_pylon/build/<rel>`).
    pub fn bundle_client(&self, app_dir: &str) -> Result<BundleClientPaths, FnCallError> {
        let timeout = *self.call_timeout.lock().unwrap();
        let mut deadline = Instant::now() + timeout;
        let hard_deadline = Instant::now() + timeout.saturating_mul(MAX_CALL_LIFETIME_MULTIPLIER);
        let started_ms = now_millis();
        let call_id = format!("b_{}", self.call_counter.fetch_add(1, Ordering::Relaxed));
        let (_route, rx) = self.register_call(&call_id)?;
        let msg = crate::protocol::BundleClientMessage::new(call_id.clone(), app_dir.to_string());
        self.send(&msg)?;
        loop {
            // Cancel a wedged client-bundle build; the child stays alive for
            // its co-tenants.
            let m = self.recv_or_cancel(
                &rx,
                deadline.min(hard_deadline),
                &call_id,
                started_ms,
                "client bundle build",
            )?;
            deadline = Instant::now() + timeout;
            match m {
                TsMessage::BundleClientResult(r) if r.call_id == call_id => {
                    if let Some(err) = r.error {
                        return Err(FnCallError {
                            code: "BUNDLE_CLIENT_FAILED".into(),
                            message: err,
                        });
                    }
                    if r.path.is_empty() {
                        return Err(FnCallError {
                            code: "BUNDLE_CLIENT_EMPTY_PATH".into(),
                            message: "Bun returned no manifest path".into(),
                        });
                    }
                    if r.outdir.is_empty() {
                        return Err(FnCallError {
                            code: "BUNDLE_CLIENT_EMPTY_OUTDIR".into(),
                            message: "Bun returned no build outdir".into(),
                        });
                    }
                    return Ok(BundleClientPaths {
                        manifest_path: r.path,
                        outdir: r.outdir,
                    });
                }
                TsMessage::Error(err) if err.call_id == call_id => {
                    return Err(FnCallError {
                        code: err.code,
                        message: err.message,
                    });
                }
                _ => {}
            }
        }
    }

    /// Lock-acquiring variant that propagates the caller's `internal`
    /// flag so the schedule hook can refuse public-to-internal smuggle
    /// attempts. Used by `FnOpsImpl::call` which knows `def.internal`
    /// from the registry; everything else goes through `call`.
    #[allow(clippy::too_many_arguments)]
    pub fn call_with_caller_internal(
        &self,
        store: &dyn DataStore,
        fn_name: &str,
        fn_type: FnType,
        args: serde_json::Value,
        auth: AuthInfo,
        on_stream: Option<StreamCallback>,
        request: Option<crate::protocol::RequestInfo>,
        stream_id: Option<String>,
        caller_internal: bool,
    ) -> Result<(serde_json::Value, crate::trace::FnTrace), FnCallError> {
        self.call_inner_with_caller_internal(
            store,
            fn_name,
            fn_type,
            args,
            auth,
            on_stream,
            request,
            stream_id,
            caller_internal,
        )
    }

    /// Protocol-only call. This is the body of a `call()`. It is `pub` so the
    /// nested-call hook in `FnOpsImpl` can re-enter the protocol for a
    /// transactional mutation wrap; it registers its OWN demux route, so it
    /// multiplexes safely alongside the parent call (no shared lock).
    ///
    /// Callers outside this crate should use `call()`; the only external caller
    /// is the nested-call hook.
    pub fn call_inner(
        &self,
        store: &dyn DataStore,
        fn_name: &str,
        fn_type: FnType,
        args: serde_json::Value,
        auth: AuthInfo,
        on_stream: Option<StreamCallback>,
        request: Option<crate::protocol::RequestInfo>,
    ) -> Result<(serde_json::Value, crate::trace::FnTrace), FnCallError> {
        // Most callers go through the lock-wrapped `call`, which goes
        // through here. Callers that need to gate scheduler enqueues
        // (public actions can't schedule internal:true targets) use
        // `call_inner_with_caller_internal` directly. Default here:
        // treat caller as public (most restrictive — public callers
        // can't enqueue internal:true targets, but public-to-public
        // works as before).
        self.call_inner_with_caller_internal(
            store, fn_name, fn_type, args, auth, on_stream, request, None, false,
        )
    }

    /// Variant of `call_inner` that takes the calling function's
    /// `internal` flag so the schedule hook can refuse public-to-
    /// internal smuggle attempts. Most callers should use `call_inner`
    /// directly; FnOpsImpl wires the per-call internal flag here.
    pub fn call_inner_with_caller_internal(
        &self,
        store: &dyn DataStore,
        fn_name: &str,
        fn_type: FnType,
        args: serde_json::Value,
        auth: AuthInfo,
        on_stream: Option<StreamCallback>,
        request: Option<crate::protocol::RequestInfo>,
        stream_id: Option<String>,
        caller_internal: bool,
    ) -> Result<(serde_json::Value, crate::trace::FnTrace), FnCallError> {
        self.call_impl(
            store,
            fn_name,
            fn_type,
            args,
            auth,
            on_stream,
            request,
            stream_id,
            caller_internal,
            None,
        )
    }

    /// Run the `agent()` action `agent_name` for `ctx.agents.run`. The
    /// call carries `invocation`, so the child refuses a target that is
    /// not an agent, checks the agent's own `auth` mode against `auth`,
    /// and hands the invocation's context to the tools. The caller (the
    /// `RunAgent` arm below, through the nested-call hook) has already
    /// checked that the requesting function may run agents as `auth`.
    pub fn call_agent_inner(
        &self,
        store: &dyn DataStore,
        agent_name: &str,
        args: serde_json::Value,
        auth: AuthInfo,
        invocation: crate::protocol::AgentInvocation,
    ) -> Result<(serde_json::Value, crate::trace::FnTrace), FnCallError> {
        self.call_impl(
            store,
            agent_name,
            FnType::Action,
            args,
            auth,
            None,
            None,
            None,
            false,
            Some(invocation),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn call_impl(
        &self,
        store: &dyn DataStore,
        fn_name: &str,
        fn_type: FnType,
        args: serde_json::Value,
        auth: AuthInfo,
        mut on_stream: Option<StreamCallback>,
        request: Option<crate::protocol::RequestInfo>,
        stream_id: Option<String>,
        caller_internal: bool,
        agent: Option<crate::protocol::AgentInvocation>,
    ) -> Result<(serde_json::Value, crate::trace::FnTrace), FnCallError> {
        // Mutable so `ctx.auth.elevate({ admin: true, ... })` can
        // promote the call mid-flight. Webhook receivers need this:
        // they're public (external systems POST to them) but want to
        // schedule internal:true workers after they've HMAC-verified
        // the request. The TS SDK emits an `ElevateAuth` message;
        // the handler arm below flips this flag, and the subsequent
        // `Schedule` arm sees the new value.
        let mut caller_is_admin = auth.is_admin;
        let caller_user_id = auth.user_id.clone();
        // The caller's identity for host hooks that act on its behalf
        // (`ctx.files.store` records it as the file's owner).
        let auth_for_hooks = auth.clone();
        let caller_tenant_id = auth.tenant_id.clone();
        let caller_is_guest = auth.is_guest;
        // Per-function `timeout` override wins over the global call timeout, so a
        // function declared long-running (heavy render, big batch) gets the time
        // it needs instead of being cancelled at the 30s default.
        //
        // The timeout is an IDLE timeout, not a wall clock: every frame the
        // call produces (stream chunk, db op, llm event) pushes the deadline
        // out by the full budget. A call actively doing work is alive — the
        // timeout exists to catch hangs, and the old absolute deadline killed
        // long agent runs mid-token-stream at exactly the moment they were
        // demonstrably making progress. `hard_deadline` is the runaway
        // backstop: no call outlives timeout × MAX_CALL_LIFETIME_MULTIPLIER
        // however chatty it is.
        let timeout = self.deadline_for(fn_name);
        let mut deadline = Instant::now() + timeout;
        let hard_deadline = Instant::now() + timeout.saturating_mul(MAX_CALL_LIFETIME_MULTIPLIER);
        let call_started_ms = now_millis();

        let call_id = format!("c_{}", self.call_counter.fetch_add(1, Ordering::Relaxed));
        // Register this call's demux route before send so its replies route to
        // THIS call's channel — even when it runs concurrently with a render or
        // another call (or is itself a nested action→query call) on the same
        // runner. `_route` unregisters + decrements the in-flight gauge on any
        // exit path (return, error, early `?`).
        let (_route, rx) = self.register_call(&call_id)?;
        let mut trace = TraceBuilder::new_with_tenant(
            call_id.clone(),
            fn_name.to_string(),
            fn_type,
            auth.user_id.clone(),
            auth.tenant_id.clone(),
        );

        // Keep an auth clone in this frame for the policy gate —
        // CallMessage::new consumes the original. Cheap (small
        // struct, mostly Option<String>).
        let gate_auth = auth.clone();

        // Send the call message. Attach HTTP request metadata when the
        // caller provided it — this lets TypeScript actions invoked via
        // /api/webhooks/:name see raw headers + body for signature checks.
        let mut call_msg =
            CallMessage::new(call_id.clone(), fn_name.to_string(), fn_type, args, auth);
        if let Some(r) = request {
            call_msg = call_msg.with_request(r);
        }
        if let Some(sid) = stream_id {
            call_msg = call_msg.with_stream_id(sid);
        }
        if let Some(invocation) = agent {
            call_msg = call_msg.with_agent(invocation);
        }
        self.send(&call_msg)?;

        // Snapshot the policy gate + the strict-mode env flag for
        // this call. Strict mode read once per call instead of per
        // op to avoid the syscall in the per-op hot path; per-call
        // is the right granularity anyway (operators flipping the
        // flag mid-deploy expect the next request to see the new
        // value, not the next ctx.db.get).
        let policy_gate_snapshot = self.policy_gate.lock().unwrap().clone();
        let strict_policies =
            std::env::var("PYLON_STRICT_FN_POLICIES").ok().as_deref() == Some("1");

        // `ctx.llm.stream` provider calls this call started. Dropped on
        // every exit path below, which cancels any still running.
        let mut llm_streams = LlmStreamOps::default();

        // Process messages until we get a return or error.
        loop {
            let msg = match recv_on(&rx, deadline.min(hard_deadline)) {
                Ok(m) => {
                    // Activity restarts the idle budget (bounded by
                    // hard_deadline above).
                    deadline = Instant::now() + timeout;
                    m
                }
                // A provider still generating is activity: the handler is
                // awaiting it, and the provider's own request timeout
                // bounds a stalled one. hard_deadline still applies.
                // So is one that just finished: the handler resumes with
                // its reply and still needs time to return.
                Err(e)
                    if e.code == "FN_TIMEOUT"
                        && (llm_streams.any_active() || llm_streams.finished_within(timeout))
                        && Instant::now() < hard_deadline =>
                {
                    deadline = Instant::now() + timeout;
                    continue;
                }
                Err(e) if e.code == "FN_TIMEOUT" => {
                    // Wedge test before deciding how to fail. A hung
                    // `await` blocks only its own call — cancel it and
                    // leave the child serving its co-tenants. But a child
                    // that emitted NOTHING (no frame from ANY call) for
                    // this call's whole lifetime has a blocked event loop:
                    // it can't even read a cancel frame, and every future
                    // call would burn its timeout too. Kill it — the
                    // supervisor respawns — restoring the old guarantee
                    // that a wedge clears within one call timeout.
                    if !self.child_emitted_since(call_started_ms) {
                        tracing::warn!(
                            "[functions] Killing TS runtime: call \"{}\" timed out with ZERO frames from the child since it started — event loop wedged",
                            fn_name
                        );
                        self.kill();
                    } else {
                        tracing::warn!(
                            "[functions] Cancelling call \"{}\": no activity within {:?} (call_id {})",
                            fn_name,
                            timeout,
                            call_id
                        );
                        let _ = self.send(&crate::protocol::CancelCallMessage::new(
                            call_id.clone(),
                            format!("idle timeout {timeout:?} exceeded"),
                        ));
                    }
                    let fn_trace = trace.finish_error(
                        "FN_TIMEOUT".into(),
                        format!("Function \"{fn_name}\" exceeded timeout {timeout:?}"),
                    );
                    self.trace_log.push(fn_trace);
                    return Err(e);
                }
                Err(e) => return Err(e),
            };
            match msg {
                TsMessage::Db(db_msg) if db_msg.call_id == call_id => {
                    let op_start = Instant::now();
                    // Per-op auth = the call's auth context, mutated
                    // mid-flight if the handler did
                    // `ctx.auth.elevate({ admin: true })`. The elevate
                    // flag flips `caller_is_admin` below; reconstruct
                    // the AuthInfo here so the policy gate sees the
                    // current (possibly elevated) state.
                    let per_op_auth = current_auth_snapshot(&gate_auth, caller_is_admin);
                    // Propagate the CURRENT admin state (elevate can flip it
                    // mid-call) to auth-aware store wrappers — the plugin
                    // chain captured its context at call entry and would
                    // otherwise reject elevated on-behalf-of writes.
                    store.set_op_admin(caller_is_admin);
                    let (result, row_count) = execute_db_op(
                        store,
                        &db_msg,
                        policy_gate_snapshot.as_deref(),
                        &per_op_auth,
                        strict_policies,
                    );
                    let duration = op_start.elapsed();
                    let ok = result.is_ok();

                    trace.record_op(
                        db_msg.op,
                        &db_msg.entity,
                        db_msg.id.as_deref(),
                        duration,
                        row_count,
                        ok,
                    );

                    // Echo op_id from the request so the TS side can demux
                    // concurrent DB ops from a single handler. Old TS
                    // runtimes that don't send op_id get the same behavior
                    // as before (one in-flight at a time, serialized by
                    // pendingRpcs key collision).
                    let reply = match result {
                        Ok(data) => {
                            DbResultMessage::ok_with_op(call_id.clone(), db_msg.op_id.clone(), data)
                        }
                        Err(e) => DbResultMessage::err_with_op(
                            call_id.clone(),
                            db_msg.op_id.clone(),
                            &e.code,
                            &e.message,
                        ),
                    };
                    self.send(&reply)?;
                }

                TsMessage::Stream(chunk) if chunk.call_id == call_id => {
                    trace.record_stream_chunk(chunk.data.len());
                    if let Some(ref mut cb) = on_stream {
                        cb(StreamChunk {
                            data: &chunk.data,
                            event: chunk.event.as_deref(),
                        });
                    }
                }

                TsMessage::Schedule(sched) if sched.call_id == call_id => {
                    trace.record_schedule(&sched.fn_name, sched.delay_ms, sched.run_at);
                    let caller = ScheduleCallerInfo {
                        caller_internal,
                        caller_is_admin,
                        caller_user_id: caller_user_id.clone(),
                        caller_tenant_id: caller_tenant_id.clone(),
                        caller_is_guest,
                    };
                    let hook_result: Result<String, String> = {
                        let hook = self.schedule_hook.lock().unwrap();
                        match *hook {
                            Some(ref cb) => cb(
                                &sched.fn_name,
                                sched.args.clone(),
                                sched.delay_ms,
                                sched.run_at,
                                caller,
                            ),
                            None => Err("no schedule hook installed".into()),
                        }
                    };
                    let reply = match hook_result {
                        Ok(id) => DbResultMessage::ok(
                            call_id.clone(),
                            serde_json::json!({"scheduled": true, "id": id}),
                        ),
                        Err(e) => DbResultMessage::err(call_id.clone(), "SCHEDULE_FAILED", &e),
                    };
                    self.send(&reply)?;
                }

                TsMessage::CancelSchedule(cancel) if cancel.call_id == call_id => {
                    let result: Option<Result<bool, String>> = {
                        let hook = self.cancel_schedule_hook.lock().unwrap();
                        hook.as_ref().map(|cb| cb(&cancel.schedule_id, store))
                    };
                    let reply = match result {
                        Some(Ok(cancelled)) => DbResultMessage::ok(
                            call_id.clone(),
                            serde_json::json!({ "cancelled": cancelled }),
                        ),
                        Some(Err(e)) => {
                            DbResultMessage::err(call_id.clone(), "SCHEDULE_CANCEL_FAILED", &e)
                        }
                        // No hook means no job queue: nothing was cancelled,
                        // and saying otherwise would hide a job that runs.
                        None => DbResultMessage::err(
                            call_id.clone(),
                            "SCHEDULE_CANCEL_FAILED",
                            "this host has no job queue wired, so the schedule cannot be cancelled",
                        ),
                    };
                    self.send(&reply)?;
                }

                TsMessage::ElevateAuth(req) if req.call_id == call_id => {
                    // Promote the per-call auth context after the
                    // handler has done its own auth check (signature
                    // verification on a webhook, JWT validation, etc.).
                    //
                    // We do NOT enforce that the developer actually
                    // verified anything — the framework can't know.
                    // The audit-log requirement (mandatory non-empty
                    // `reason`) makes every elevation traceable so
                    // a misuse is at least findable post-incident.
                    //
                    // Today: only `admin` is supported. Tomorrow could
                    // also flip caller_internal, set a synthetic user
                    // id, etc. — keep this single-field for now so the
                    // semantics are easy to reason about.
                    let reply = if req.reason.trim().is_empty() {
                        DbResultMessage::err(
                            call_id.clone(),
                            "ELEVATE_NO_REASON",
                            "elevate({ reason }) requires a non-empty reason — every privilege escalation must be auditable",
                        )
                    } else if !req.admin {
                        // No-op elevation request. Still reply OK so
                        // future-compatible callers don't error when
                        // they pass `admin: false` to revert (not yet
                        // supported, but reserve the shape).
                        DbResultMessage::ok(call_id.clone(), serde_json::json!({"elevated": false}))
                    } else {
                        // Promote. Log at INFO with fn name + reason
                        // so operators have an audit trail without
                        // having to plumb a dedicated table.
                        tracing::info!(
                            "[functions] elevate_auth: fn=\"{}\" admin=true reason=\"{}\"",
                            fn_name,
                            req.reason
                        );
                        caller_is_admin = true;
                        DbResultMessage::ok(call_id.clone(), serde_json::json!({"elevated": true}))
                    };
                    self.send(&reply)?;
                }

                TsMessage::SignFileUrl(req) if req.call_id == call_id => {
                    // `ctx.files.signedUrl` — mint an HMAC-signed download
                    // path. Authorization for WHO may receive the URL is the
                    // calling function's job (membership gates); the runner
                    // just signs.
                    let result: Option<Result<String, (String, String)>> = {
                        let signer = self.file_url_signer.lock().unwrap();
                        signer.as_ref().map(|cb| cb(&req.file_id, req.ttl_secs))
                    };
                    let reply = match result {
                        Some(Ok(url)) => {
                            DbResultMessage::ok(call_id.clone(), serde_json::json!(url))
                        }
                        Some(Err((code, msg))) => {
                            DbResultMessage::err(call_id.clone(), &code, &msg)
                        }
                        None => DbResultMessage::err(
                            call_id.clone(),
                            "FILES_SIGNING_NOT_CONFIGURED",
                            "this host does not support signed file URLs",
                        ),
                    };
                    self.send(&reply)?;
                }

                TsMessage::FilesOp(req) if req.call_id == call_id => {
                    let reply = if fn_type != FnType::Action {
                        DbResultMessage::err(
                            call_id.clone(),
                            "FILES_WRITE_NOT_ALLOWED",
                            "ctx.files.store and ctx.files.delete run only in actions: a file write cannot roll back with a query or mutation",
                        )
                    } else {
                        let auth_now = AuthInfo {
                            is_admin: caller_is_admin,
                            ..auth_for_hooks.clone()
                        };
                        let result = {
                            let hook = self.files_op_hook.lock().unwrap().clone();
                            hook.as_ref().map(|cb| cb(&req.op, &auth_now))
                        };
                        match result {
                            Some(Ok(v)) => DbResultMessage::ok(call_id.clone(), v),
                            Some(Err((code, msg))) => {
                                DbResultMessage::err(call_id.clone(), &code, &msg)
                            }
                            None => DbResultMessage::err(
                                call_id.clone(),
                                "FILES_NOT_CONFIGURED",
                                "this host does not support file storage from functions",
                            ),
                        }
                    };
                    self.send(&reply)?;
                }

                TsMessage::SignShardTicket(req) if req.call_id == call_id => {
                    // `ctx.shards.ticket` — the calling function decides who
                    // gets a ticket for what; the runner just signs.
                    let result: Option<Result<String, (String, String)>> = {
                        let signer = self.shard_ticket_signer.lock().unwrap();
                        signer
                            .as_ref()
                            .map(|cb| cb(&req, caller_user_id.as_deref()))
                    };
                    let reply = match result {
                        Some(Ok(ticket)) => {
                            DbResultMessage::ok(call_id.clone(), serde_json::json!(ticket))
                        }
                        Some(Err((code, msg))) => {
                            DbResultMessage::err(call_id.clone(), &code, &msg)
                        }
                        None => DbResultMessage::err(
                            call_id.clone(),
                            "SHARD_TICKETS_NOT_CONFIGURED",
                            "this host does not sign shard tickets",
                        ),
                    };
                    self.send(&reply)?;
                }

                TsMessage::ShardOp(req) if req.call_id == call_id => {
                    // Starting or stopping a shard is a side effect outside
                    // the database: a query re-runs whenever its reads
                    // change, and a mutation's rollback cannot undo it.
                    // A mutation's `send` is held until its COMMIT (the
                    // hook buffers it); a query has no commit.
                    let reply = if !matches!(fn_type, crate::protocol::FnType::Action)
                        && matches!(req.op.as_str(), "create" | "stop" | "transfer" | "publish")
                    {
                        DbResultMessage::err(
                            call_id.clone(),
                            "SHARD_OP_ACTIONS_ONLY",
                            "ctx.shards.create, stop, transfer, and publish are available in actions only (a mutation's rollback cannot undo them)",
                        )
                    } else if matches!(fn_type, crate::protocol::FnType::Query) && req.op == "send"
                    {
                        DbResultMessage::err(
                            call_id.clone(),
                            "SHARD_OP_NOT_IN_QUERY",
                            "ctx.shards.send is available in mutations and actions (a query re-runs whenever its reads change)",
                        )
                    } else {
                        let hook = self
                            .shard_op_hook
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .clone();
                        let result = hook.map(|cb| cb(&req));
                        match result {
                            Some(Ok(value)) => DbResultMessage::ok(call_id.clone(), value),
                            Some(Err((code, msg))) => {
                                DbResultMessage::err(call_id.clone(), &code, &msg)
                            }
                            None => DbResultMessage::err(
                                call_id.clone(),
                                "SHARDS_NOT_CONFIGURED",
                                "this app declares no shards; add `shards: [shard({ name, wasm })]` to buildManifest in app.ts",
                            ),
                        }
                    };
                    self.send(&reply)?;
                }

                TsMessage::AuditOp(req) if req.call_id == call_id => {
                    let auth_now = current_auth_snapshot(&gate_auth, caller_is_admin);
                    let result: Option<Result<serde_json::Value, (String, String)>> = {
                        let hook = self.audit_op_hook.lock().unwrap();
                        hook.as_ref().map(|cb| cb(&req, &auth_now))
                    };
                    let reply = match result {
                        Some(Ok(value)) => DbResultMessage::ok(call_id.clone(), value),
                        Some(Err((code, msg))) => {
                            DbResultMessage::err(call_id.clone(), &code, &msg)
                        }
                        None => DbResultMessage::err(
                            call_id.clone(),
                            "AUDIT_NOT_CONFIGURED",
                            "this host has no audit log wired",
                        ),
                    };
                    self.send(&reply)?;
                }

                TsMessage::WorkflowOp(req) if req.call_id == call_id => {
                    // `ctx.workflows.start` / `.sendEvent` — app code
                    // driving durable workflows. Trust model matches
                    // ctx.scheduler: server-side handler code is trusted
                    // to start its own app's workflows.
                    let result: Option<Result<serde_json::Value, (String, String)>> = {
                        let hook = self.workflow_op_hook.lock().unwrap();
                        hook.as_ref().map(|cb| cb(&req))
                    };
                    let reply = match result {
                        Some(Ok(value)) => DbResultMessage::ok(call_id.clone(), value),
                        Some(Err((code, msg))) => {
                            DbResultMessage::err(call_id.clone(), &code, &msg)
                        }
                        None => DbResultMessage::err(
                            call_id.clone(),
                            "WORKFLOWS_NOT_CONFIGURED",
                            "this host has no workflow engine wired (no workflows/ dir was declared)",
                        ),
                    };
                    self.send(&reply)?;
                }

                TsMessage::RunFn(run) if run.call_id == call_id => {
                    // Nested function call (action calling query/mutation).
                    // Execute recursively. The nested call gets its own trace
                    // and inherits the caller's current admin state, roles,
                    // user, and tenant. This keeps authorization consistent
                    // with other host operations after an in-call elevation.
                    let nested_auth = current_auth_snapshot(&gate_auth, caller_is_admin);
                    // Prefer the nested_call_hook if installed — it lets the
                    // caller wrap mutations in their own BEGIN/COMMIT around
                    // a TxStore. Falling back to direct recursion leaves
                    // mutations non-transactional when triggered from an
                    // action (documented limitation).
                    let hook_result: Option<Result<serde_json::Value, (String, String)>> = {
                        let hook = self.nested_call_hook.lock().unwrap().clone();
                        hook.map(|cb| {
                            cb(
                                &run.fn_name,
                                run.fn_type,
                                run.args.clone(),
                                nested_auth.clone(),
                                None,
                            )
                        })
                    };
                    let reply = match hook_result {
                        Some(Ok(value)) => DbResultMessage::ok(call_id.clone(), value),
                        Some(Err((code, msg))) => {
                            DbResultMessage::err(call_id.clone(), &code, &msg)
                        }
                        None => {
                            // No hook installed — fall back to direct recursion.
                            // The nested call_inner registers its OWN demux route
                            // (a fresh call_id), so it multiplexes correctly even
                            // though the parent call's route is still open: the
                            // parent's TS side is blocked awaiting this reply, so
                            // no parent-routed message arrives meanwhile. Nested
                            // calls never get HTTP request metadata.
                            match self.call_inner(
                                store,
                                &run.fn_name,
                                run.fn_type,
                                run.args,
                                nested_auth,
                                None,
                                None,
                            ) {
                                Ok((value, _nested_trace)) => {
                                    DbResultMessage::ok(call_id.clone(), value)
                                }
                                Err(e) => DbResultMessage::err(
                                    call_id.clone(),
                                    "FN_CALL_FAILED",
                                    &e.message,
                                ),
                            }
                        }
                    };
                    // The nested call ran on this thread, so this call's
                    // idle budget was not renewed meanwhile. Renew it now,
                    // or a nested call longer than the caller's timeout
                    // times the caller out the moment it returns.
                    deadline = Instant::now() + timeout;
                    self.send(&reply)?;
                }

                TsMessage::RunAgent(req) if req.call_id == call_id => {
                    let reply = match run_agent_auth(
                        &req,
                        fn_type,
                        caller_internal,
                        caller_is_admin,
                    ) {
                        Err((code, msg)) => DbResultMessage::err(call_id.clone(), code, &msg),
                        Ok(agent_auth) => {
                            tracing::info!(
                                "[functions] run_agent: fn=\"{}\" agent=\"{}\" as_user=\"{}\" admin={}",
                                fn_name,
                                req.agent,
                                req.user_id,
                                req.admin
                            );
                            let invocation = crate::protocol::AgentInvocation {
                                context: req.context.clone(),
                            };
                            let hook_result: Option<Result<serde_json::Value, (String, String)>> = {
                                let hook = self.nested_call_hook.lock().unwrap().clone();
                                hook.map(|cb| {
                                    cb(
                                        &req.agent,
                                        FnType::Action,
                                        req.args.clone(),
                                        agent_auth.clone(),
                                        Some(invocation.clone()),
                                    )
                                })
                            };
                            let result = match hook_result {
                                Some(r) => r,
                                None => self
                                    .call_agent_inner(
                                        store, &req.agent, req.args, agent_auth, invocation,
                                    )
                                    .map(|(value, _)| value)
                                    .map_err(|e| (e.code, e.message)),
                            };
                            match result {
                                Ok(value) => DbResultMessage::ok(call_id.clone(), value),
                                Err((code, msg)) => {
                                    DbResultMessage::err(call_id.clone(), &code, &msg)
                                }
                            }
                        }
                    };
                    // Same as RunFn: the agent ran on this thread.
                    deadline = Instant::now() + timeout;
                    self.send(&reply)?;
                }

                TsMessage::LlmComplete(req) if req.call_id == call_id => {
                    // Codex P1-12 (host-side enforcement): queries are
                    // reactive — re-runs would re-bill the LLM call on
                    // every dep change AND violate the reactive purity
                    // contract. Refuse here even if a buggy TS handler
                    // tries to send the message from a query ctx.
                    if matches!(fn_type, crate::protocol::FnType::Query) {
                        let reply = DbResultMessage::err(
                            call_id.clone(),
                            "LLM_NOT_AVAILABLE_IN_QUERY",
                            "ctx.llm.complete is not available in query handlers (queries are reactive — LLM calls belong in mutations or actions).",
                        );
                        self.send(&reply)?;
                        continue;
                    }
                    // Build an AuthInfo snapshot for the hook so it can
                    // enforce per-user model gating / spend accounting.
                    let auth_snapshot = current_auth_snapshot(&gate_auth, caller_is_admin);
                    let result: Result<serde_json::Value, (String, String)> = {
                        let hook = self.llm_hook.lock().unwrap().clone();
                        match hook {
                            Some(ref cb) => cb(&req.request, &auth_snapshot),
                            None => Err((
                                "LLM_NOT_CONFIGURED".into(),
                                "ctx.llm.complete: no LLM provider configured (set PYLON_LLM_PROVIDER + API key)".into(),
                            )),
                        }
                    };
                    let reply = match result {
                        Ok(value) => DbResultMessage::ok(call_id.clone(), value),
                        Err((code, msg)) => DbResultMessage::err(call_id.clone(), &code, &msg),
                    };
                    self.send(&reply)?;
                }

                TsMessage::LlmEmbed(req) if req.call_id == call_id => {
                    // Embeddings share the reactive-purity refusal:
                    // queries re-run on every dep change, which would
                    // re-bill the provider each time. Embed in an
                    // action, store the vector, query the stored rows.
                    if matches!(fn_type, crate::protocol::FnType::Query) {
                        let reply = DbResultMessage::err_with_op(
                            call_id.clone(),
                            req.op_id.clone(),
                            "LLM_NOT_AVAILABLE_IN_QUERY",
                            "ctx.llm.embed is not available in query handlers (queries are reactive — embed in a mutation or action and store the result).",
                        );
                        self.send(&reply)?;
                        continue;
                    }
                    let auth_snapshot = current_auth_snapshot(&gate_auth, caller_is_admin);
                    let result: Result<serde_json::Value, (String, String)> = {
                        let hook = self.llm_embed_hook.lock().unwrap().clone();
                        match hook {
                            Some(ref cb) => cb(&req.request, &auth_snapshot),
                            None => Err((
                                "EMBEDDINGS_NOT_CONFIGURED".into(),
                                "ctx.llm.embed: no embeddings provider configured (set OPENAI_API_KEY, or PYLON_EMBEDDINGS_PROVIDER=voyage + VOYAGE_API_KEY)".into(),
                            )),
                        }
                    };
                    let reply = match result {
                        Ok(value) => {
                            DbResultMessage::ok_with_op(call_id.clone(), req.op_id.clone(), value)
                        }
                        Err((code, msg)) => DbResultMessage::err_with_op(
                            call_id.clone(),
                            req.op_id.clone(),
                            &code,
                            &msg,
                        ),
                    };
                    self.send(&reply)?;
                }

                TsMessage::LlmStream(req) if req.call_id == call_id => {
                    // Same reactive-purity refusal as LlmComplete:
                    // a query re-runs on every dep change, which
                    // would re-bill the provider each time.
                    if matches!(fn_type, crate::protocol::FnType::Query) {
                        let reply = DbResultMessage::err_with_op(
                            call_id.clone(),
                            req.op_id.clone(),
                            "LLM_NOT_AVAILABLE_IN_QUERY",
                            "ctx.llm.stream is not available in query handlers (queries are reactive — LLM calls belong in mutations or actions).",
                        );
                        self.send(&reply)?;
                        continue;
                    }
                    let hook = self.llm_stream_hook.lock().unwrap().clone();
                    let Some(hook) = hook else {
                        let reply = DbResultMessage::err_with_op(
                            call_id.clone(),
                            req.op_id.clone(),
                            "LLM_NOT_CONFIGURED",
                            "ctx.llm.stream: no LLM provider configured (set PYLON_LLM_PROVIDER + API key)",
                        );
                        self.send(&reply)?;
                        continue;
                    };
                    // The provider call runs on its own thread so this
                    // loop keeps serving the handler while tokens arrive:
                    // the `ctx.stream.write` frames the handler sends from
                    // its onEvent callback reach the client as they are
                    // written, and ctx.db / runQuery calls made during the
                    // generation are answered. The thread writes the
                    // `llm_event` frames and the terminal reply itself.
                    let cancel = Arc::new(AtomicBool::new(false));
                    llm_streams
                        .ops
                        .push((req.op_id.clone(), Arc::clone(&cancel)));
                    llm_streams.active.fetch_add(1, Ordering::SeqCst);
                    let active = ActiveStreamGuard(
                        Arc::clone(&llm_streams.active),
                        Arc::clone(&llm_streams.last_finished_ms),
                    );
                    let stdin = Arc::clone(&self.stdin);
                    let auth_snapshot = current_auth_snapshot(&gate_auth, caller_is_admin);
                    let stream_call_id = call_id.clone();
                    let op_id = req.op_id.clone();
                    let spawned = std::thread::Builder::new()
                        .name("pylon-llm-stream".into())
                        .spawn(move || {
                            let _active = active;
                            // Nothing is written once the stream is
                            // cancelled: the handler has stopped waiting
                            // (llm_cancel), or the call is over and its
                            // route is gone.
                            let mut on_event = |event: serde_json::Value| {
                                if cancel.load(Ordering::SeqCst) {
                                    return;
                                }
                                let msg = crate::protocol::LlmEventMessage::new(
                                    stream_call_id.clone(),
                                    op_id.clone(),
                                    event,
                                );
                                let _ = write_frame(&stdin, &msg);
                            };
                            let result = hook(&req.request, &auth_snapshot, &mut on_event, &cancel);
                            if cancel.load(Ordering::SeqCst) {
                                return;
                            }
                            let reply = match result {
                                Ok(value) => {
                                    DbResultMessage::ok_with_op(stream_call_id, op_id, value)
                                }
                                Err((code, msg)) => {
                                    DbResultMessage::err_with_op(stream_call_id, op_id, &code, &msg)
                                }
                            };
                            let _ = write_frame(&stdin, &reply);
                        });
                    if let Err(e) = spawned {
                        let reply = DbResultMessage::err_with_op(
                            call_id.clone(),
                            req.op_id.clone(),
                            "LLM_STREAM_FAILED",
                            &format!("could not start the stream thread: {e}"),
                        );
                        self.send(&reply)?;
                    }
                }

                TsMessage::LlmCancel(req) if req.call_id == call_id => {
                    // The handler stopped waiting on one stream (its
                    // AbortSignal fired). Cancelling drops the provider
                    // response, which closes the connection so no more
                    // tokens are generated. No reply: the handler has
                    // already settled its promise.
                    llm_streams.cancel(req.op_id.as_deref());
                }

                TsMessage::RoomBroadcast(req) if req.call_id == call_id => {
                    let result: Result<bool, (String, String)> = {
                        let hook = self.room_broadcast_hook.lock().unwrap();
                        match *hook {
                            Some(ref cb) => cb(&req.room, &req.topic, req.data.clone()),
                            None => Err((
                                "ROOMS_NOT_CONFIGURED".into(),
                                "ctx.rooms.broadcast: the runtime exposed no room manager".into(),
                            )),
                        }
                    };
                    let reply = match result {
                        Ok(delivered) => DbResultMessage::ok_with_op(
                            call_id.clone(),
                            req.op_id.clone(),
                            serde_json::json!({ "delivered": delivered }),
                        ),
                        Err((code, msg)) => DbResultMessage::err_with_op(
                            call_id.clone(),
                            req.op_id.clone(),
                            &code,
                            &msg,
                        ),
                    };
                    self.send(&reply)?;
                }

                TsMessage::Connection(req) if req.call_id == call_id => {
                    // Refuse unauthenticated callers. ctx.connections.*
                    // is bound to ctx.auth.userId — without a user
                    // identity there's nothing to look up.
                    if caller_user_id.is_none() && !caller_is_admin {
                        let reply = DbResultMessage::err(
                            call_id.clone(),
                            "CONNECTION_REQUIRES_AUTH",
                            "ctx.connections.* requires an authenticated user. Public callers must ctx.auth.elevate first.",
                        );
                        self.send(&reply)?;
                        continue;
                    }
                    let auth_snapshot = current_auth_snapshot(&gate_auth, caller_is_admin);
                    let result: Result<serde_json::Value, (String, String)> = {
                        let hook = self.connection_hook.lock().unwrap().clone();
                        match hook {
                            Some(ref cb) => cb(&req.op, &req.payload, &auth_snapshot),
                            None => Err((
                                "CONNECTIONS_NOT_CONFIGURED".into(),
                                "No defineConnection(...) entries in the manifest.".into(),
                            )),
                        }
                    };
                    let reply = match result {
                        Ok(value) => DbResultMessage::ok(call_id.clone(), value),
                        Err((code, msg)) => DbResultMessage::err(call_id.clone(), &code, &msg),
                    };
                    self.send(&reply)?;
                }

                TsMessage::SendEmail(req) if req.call_id == call_id => {
                    // Size guard mirrors the TS-side check: attachments ride
                    // one NDJSON line, so an oversized payload is an
                    // unbounded allocation on both sides of the pipe.
                    let b64_total: usize = req.attachments.iter().map(|a| a.content.len()).sum();
                    if req.attachments.len() > EMAIL_MAX_ATTACHMENTS
                        || b64_total > EMAIL_MAX_ATTACHMENT_B64_BYTES
                    {
                        let reply = DbResultMessage::err(
                            call_id.clone(),
                            "EMAIL_TOO_LARGE",
                            &format!(
                                "email exceeds limits: {} attachments ({} base64 bytes); max {} attachments, {} bytes total",
                                req.attachments.len(),
                                b64_total,
                                EMAIL_MAX_ATTACHMENTS,
                                EMAIL_MAX_ATTACHMENT_B64_BYTES
                            ),
                        );
                        self.send(&reply)?;
                        continue;
                    }
                    // Hand off to the runtime's email transport (configured
                    // via PYLON_EMAIL_PROVIDER). Without a hook installed
                    // we surface the missing-config gap explicitly so
                    // operators don't think their invite emails sent.
                    let message = pylon_kernel::EmailMessage {
                        to: req.to.clone(),
                        subject: req.subject.clone(),
                        text: req.body.clone(),
                        html: req.html.clone(),
                        attachments: req.attachments.clone(),
                    };
                    let result: Result<(), String> = {
                        let hook = self.email_hook.lock().unwrap().clone();
                        match hook {
                            Some(ref cb) => cb(&message),
                            None => Err(
                                "ctx.email.send: no email transport configured (set PYLON_EMAIL_PROVIDER)".into(),
                            ),
                        }
                    };
                    let reply = match result {
                        Ok(()) => {
                            DbResultMessage::ok(call_id.clone(), serde_json::json!({"sent": true}))
                        }
                        Err(e) => DbResultMessage::err(call_id.clone(), "EMAIL_SEND_FAILED", &e),
                    };
                    self.send(&reply)?;
                }

                TsMessage::Return(ret) if ret.call_id == call_id => {
                    let fn_trace = trace.finish_ok_ref(&ret.value);
                    self.trace_log.push(fn_trace.clone());
                    return Ok((ret.value, fn_trace));
                }

                TsMessage::Error(err) if err.call_id == call_id => {
                    let fn_trace = trace.finish_error_ref(&err.code, &err.message);
                    self.trace_log.push(fn_trace);
                    return Err(FnCallError {
                        code: err.code,
                        message: err.message,
                    });
                }

                // Messages for a different call_id — shouldn't happen with
                // sequential execution, but skip gracefully.
                _ => {}
            }
        }
    }

    fn send<T: serde::Serialize>(&self, msg: &T) -> Result<(), FnCallError> {
        write_frame(&self.stdin, msg)
    }

    /// Register a demux route for `call_id` and return its receiver + an RAII
    /// guard. The guard unregisters the route (and decrements the in-flight
    /// gauge) when dropped — on return, error, or panic — so a finished call
    /// never leaves a dangling route. MUST be called before `send()` so the
    /// reader can route the very first reply.
    fn register_call(
        &self,
        call_id: &str,
    ) -> Result<(CallRoute, Receiver<TsMessage>), FnCallError> {
        let table = self
            .routes
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| FnCallError {
                code: "RUNNER_NOT_STARTED".into(),
                message: "TypeScript function runner is not running".into(),
            })?;
        let (tx, rx) = mpsc::channel();
        table.lock().unwrap().insert(call_id.to_string(), tx);
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        // Stamp busy_since (NOT last_msg_at). The probe measures silence
        // from max(last_msg_at, busy_since), so a runner that sat idle past
        // the probe window and then takes a call doesn't read as wedged on
        // the first probe — while last_msg_at itself stays a truthful
        // "last frame FROM the child" signal. Resetting last_msg_at here
        // (the old code) let a steady stream of NEW calls keep a fully
        // wedged child looking alive forever: every arrival reset the
        // silence clock the probe reads, `pick()` kept routing into the
        // wedge, and every call burned its timeout — the homepage-outage
        // pattern.
        self.busy_since.store(now_millis(), Ordering::Relaxed);
        Ok((
            CallRoute {
                table,
                gauge: Arc::clone(&self.in_flight),
                call_id: call_id.to_string(),
            },
            rx,
        ))
    }

    /// Receive the next message for a call from ITS demux channel, CANCELLING
    /// the call on a timeout — or KILLING the child when the timeout looks
    /// like an event-loop wedge.
    ///
    /// This used to always kill the whole child on timeout, on the theory
    /// that a deadline-exceeded meant a wedged event loop. With multiplexing
    /// that killed every co-tenant call and SSR render for one slow handler —
    /// and the theory was wrong for the common case: a hung `await` blocks
    /// nothing but its own call. The wedge test is `started_ms`: if the child
    /// emitted NO frame at all since this op began, its event loop is blocked
    /// (it couldn't read a cancel frame either) and the kill is the only
    /// recovery; otherwise cancel just this call. `label` names the op for
    /// the log.
    fn recv_or_cancel(
        &self,
        rx: &Receiver<TsMessage>,
        deadline: Instant,
        call_id: &str,
        started_ms: u64,
        label: &str,
    ) -> Result<TsMessage, FnCallError> {
        match recv_on(rx, deadline) {
            Ok(m) => Ok(m),
            Err(e) if e.code == "FN_TIMEOUT" => {
                if !self.child_emitted_since(started_ms) {
                    tracing::warn!(
                        "[functions] Killing TS runtime: {label} (call_id {call_id}) timed out with ZERO frames from the child — event loop wedged"
                    );
                    self.kill();
                } else {
                    tracing::warn!(
                        "[functions] Cancelling {label} (call_id {call_id}): exceeded its call timeout"
                    );
                    let _ = self.send(&crate::protocol::CancelCallMessage::new(
                        call_id.to_string(),
                        format!("{label} exceeded its call timeout"),
                    ));
                }
                Err(e)
            }
            Err(e) => Err(e),
        }
    }
}

/// Who `ctx.agents.run` may run an agent as, or why it may not.
///
/// Server code only: the requesting function must be an action that is
/// either `internal: true` (no client can call it; jobs, workflows, and
/// other server code start it) or running with admin rights (an admin
/// session, or a webhook after `ctx.auth.elevate`). A public action a
/// client calls cannot pick an identity to act as. `admin: true` needs an
/// admin caller, so an internal non-admin function cannot mint admin
/// rights.
fn run_agent_auth(
    req: &crate::protocol::RunAgentMessage,
    fn_type: FnType,
    caller_internal: bool,
    caller_is_admin: bool,
) -> Result<AuthInfo, (&'static str, String)> {
    if !matches!(fn_type, FnType::Action) {
        return Err((
            "AGENT_RUN_ACTIONS_ONLY",
            "ctx.agents.run is available in actions only".into(),
        ));
    }
    if !caller_internal && !caller_is_admin {
        return Err((
            "AGENT_RUN_FORBIDDEN",
            "ctx.agents.run needs a server-side caller: an internal: true action, or an admin one (a webhook can call ctx.auth.elevate first)".into(),
        ));
    }
    if req.admin && !caller_is_admin {
        return Err((
            "AGENT_RUN_FORBIDDEN",
            "ctx.agents.run with as.admin needs a caller that is an admin".into(),
        ));
    }
    let user_id = req.user_id.trim();
    if user_id.is_empty() {
        return Err((
            "INVALID_ARGS",
            "ctx.agents.run needs as.userId: the user the run belongs to".into(),
        ));
    }
    if req.agent.trim().is_empty() {
        return Err(("INVALID_ARGS", "ctx.agents.run needs an agent name".into()));
    }
    Ok(AuthInfo {
        user_id: Some(user_id.to_string()),
        is_admin: req.admin,
        tenant_id: None,
        roles: vec![],
        is_guest: false,
    })
}

/// Serialize `msg` as one NDJSON line and write it to the child's stdin.
/// The lock keeps concurrent writers (call loops, stream threads) from
/// interleaving bytes within a frame.
fn write_frame<T: serde::Serialize>(
    stdin: &Mutex<Option<std::process::ChildStdin>>,
    msg: &T,
) -> Result<(), FnCallError> {
    let mut line = serde_json::to_string(msg).map_err(|e| FnCallError {
        code: "SERIALIZE_FAILED".into(),
        message: format!("Failed to serialize message: {e}"),
    })?;
    line.push('\n');

    let mut stdin_guard = stdin.lock().unwrap();
    let stdin = stdin_guard.as_mut().ok_or_else(|| FnCallError {
        code: "RUNNER_NOT_STARTED".into(),
        message: "TypeScript function runner is not running".into(),
    })?;
    stdin.write_all(line.as_bytes()).map_err(|e| FnCallError {
        code: "IO_ERROR".into(),
        message: format!("Failed to write to runner: {e}"),
    })?;
    stdin.flush().map_err(|e| FnCallError {
        code: "IO_ERROR".into(),
        message: format!("Failed to flush runner stdin: {e}"),
    })?;
    Ok(())
}

/// The `ctx.llm.stream` provider calls one function call has started.
/// Each runs on its own thread. This tracks them so the call can cancel
/// one by op id (`llm_cancel` from the handler), hold off its idle
/// timeout while a provider is still generating, and cancel all of them
/// when the call ends for any reason (the drop).
#[derive(Default)]
struct LlmStreamOps {
    ops: Vec<(Option<String>, Arc<AtomicBool>)>,
    /// Streams whose thread has not finished yet.
    active: Arc<AtomicUsize>,
    /// When the last stream thread finished (ms epoch, 0 = none yet).
    last_finished_ms: Arc<AtomicU64>,
}

impl LlmStreamOps {
    fn cancel(&self, op_id: Option<&str>) {
        for (id, flag) in &self.ops {
            if id.as_deref() == op_id {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }

    fn any_active(&self) -> bool {
        self.active.load(Ordering::SeqCst) > 0
    }

    /// A stream finished less than `window` ago.
    fn finished_within(&self, window: Duration) -> bool {
        let at = self.last_finished_ms.load(Ordering::SeqCst);
        at != 0 && now_millis().saturating_sub(at) < window.as_millis() as u64
    }
}

impl Drop for LlmStreamOps {
    fn drop(&mut self) {
        for (_, flag) in &self.ops {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

/// Decrements the active-stream count and records the finish time when a
/// stream thread exits, including by panic.
struct ActiveStreamGuard(Arc<AtomicUsize>, Arc<AtomicU64>);

impl Drop for ActiveStreamGuard {
    fn drop(&mut self) {
        self.1.store(now_millis(), Ordering::SeqCst);
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// How long `start` waits for the runtime's Ready message.
/// `PYLON_FN_READY_TIMEOUT_SECS`, default 30 s: a large app importing many
/// functions on a cold machine can take longer than a few seconds.
fn ready_timeout_from_env() -> Duration {
    std::env::var("PYLON_FN_READY_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30))
}

/// Block up to `deadline` for the next message on a call's demux channel.
/// A timeout maps to `FN_TIMEOUT` (the call exceeded its budget) and a
/// disconnected channel to `RUNNER_EXITED` (the reader cleared routes because
/// the child died).
fn recv_on(rx: &Receiver<TsMessage>, deadline: Instant) -> Result<TsMessage, FnCallError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    match rx.recv_timeout(remaining) {
        Ok(msg) => Ok(msg),
        Err(RecvTimeoutError::Timeout) => Err(FnCallError {
            code: "FN_TIMEOUT".into(),
            message: "Function exceeded the configured call timeout".into(),
        }),
        Err(RecvTimeoutError::Disconnected) => Err(FnCallError {
            code: "RUNNER_EXITED".into(),
            message: "TypeScript function runner process exited unexpectedly".into(),
        }),
    }
}

/// Kill a child and pass through an error message — used during start()
/// when something goes wrong after spawn but before publishing the IO.
/// Always wait() after kill() so the child is reaped — otherwise it
/// hangs around as a zombie until the parent exits.
fn kill_and_msg(child: &mut Child, msg: String) -> String {
    let _ = child.kill();
    let _ = child.wait();
    msg
}

/// Reader thread: parse each NDJSON line and DEMUX it to the waiting call by
/// `call_id`. The one-shot `Ready` (no call_id) goes to `ready_tx`. Every other
/// message is routed to its call's channel; a message for an unknown call_id
/// (a late frame after the call unregistered) is dropped. On child exit the
/// table is cleared so every in-flight call's recv returns `Disconnected`
/// (→ `RUNNER_EXITED`) instead of hanging until its deadline.
/// Max bytes buffered for ONE NDJSON frame. A frame with no terminating
/// newline (a malformed / hostile / pathologically-large return value) would
/// otherwise grow the read buffer without bound and OOM the host — which kills
/// EVERY runner, not just the offending call. Beyond this the frame is dropped
/// and the reader re-syncs on the next newline; the waiting call then times out
/// gracefully instead. 64 MiB is far above any legitimate single frame (render
/// chunks stream in small pieces; the largest normal frame is an action's JSON
/// return).
const MAX_FRAME_BYTES: u64 = 64 * 1024 * 1024;

enum Frame {
    /// A complete NDJSON line (bytes include the trailing '\n' except at EOF).
    Data(Vec<u8>),
    /// A frame exceeding `MAX_FRAME_BYTES` with no newline — dropped + drained.
    Oversized,
    /// Child stdout reached EOF.
    Eof,
}

/// Read one newline-terminated frame from `r`, bounded to `max` bytes. An
/// oversized frame is dropped and its tail drained so the NEXT call re-syncs on
/// a clean line boundary.
fn read_frame<R: BufRead>(r: &mut R, max: u64) -> std::io::Result<Frame> {
    let mut buf = Vec::new();
    let n = (&mut *r).take(max).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(Frame::Eof);
    }
    if buf.last() == Some(&b'\n') {
        return Ok(Frame::Data(buf));
    }
    // No terminating newline within what we read.
    if buf.len() as u64 >= max {
        // Cap hit → oversized/hostile frame. Discard its remaining bytes
        // (without buffering them) so the reader picks up the next frame.
        drain_to_newline(r)?;
        return Ok(Frame::Oversized);
    }
    // A partial final line the child wrote before exiting — hand it up as-is.
    Ok(Frame::Data(buf))
}

/// Discard bytes up to and including the next '\n' (or EOF) WITHOUT buffering
/// them. Returns Ok(true) if a newline was consumed, Ok(false) on EOF.
fn drain_to_newline<R: BufRead>(r: &mut R) -> std::io::Result<bool> {
    loop {
        let available = r.fill_buf()?;
        if available.is_empty() {
            return Ok(false); // EOF
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            r.consume(pos + 1);
            return Ok(true);
        }
        let consumed = available.len();
        r.consume(consumed);
    }
}

fn reader_loop(
    mut stdout: BufReader<std::process::ChildStdout>,
    routes: Arc<RouteTable>,
    ready_tx: Sender<TsMessage>,
    last_msg_at: Arc<AtomicU64>,
) {
    loop {
        let buf = match read_frame(&mut stdout, MAX_FRAME_BYTES) {
            Ok(Frame::Eof) => break, // EOF — child exited
            Err(_) => break,         // pipe error — child gone
            Ok(Frame::Data(buf)) => buf,
            Ok(Frame::Oversized) => {
                tracing::warn!(
                    "[functions] dropping oversized NDJSON frame (>= {MAX_FRAME_BYTES} bytes, no newline) — the waiting call will time out"
                );
                continue;
            }
        };
        // `from_slice` tolerates the trailing newline (whitespace). Bytes, not a
        // String, so a non-UTF-8 frame is a parse error we skip — not a panic.
        match serde_json::from_slice::<TsMessage>(&buf) {
            Ok(msg) => {
                // Liveness: a message just arrived, so the runtime is
                // responsive. Feeds the health probe (busy-but-flowing = OK).
                last_msg_at.store(now_millis(), Ordering::Relaxed);
                route_message(&routes, &ready_tx, msg);
            }
            Err(e) => {
                tracing::warn!(
                    "[functions] Skipping unparseable line from Bun runtime: {e} (line={:?})",
                    String::from_utf8_lossy(&buf).trim()
                );
            }
        }
    }
    // Child gone — disconnect every in-flight call so its recv returns
    // Disconnected (RUNNER_EXITED) immediately, rather than blocking until the
    // call timeout.
    if let Ok(mut g) = routes.lock() {
        g.clear();
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Route ONE parsed message to its destination. The security-critical core of
/// multiplexing: a message is delivered ONLY to the channel registered under
/// its own `call_id`, so a render/function can never receive another call's
/// output. The call_id-less `Ready` goes to the handshake channel; a message
/// for an unregistered id (a late frame after the call unwound) is dropped.
fn route_message(routes: &RouteTable, ready_tx: &Sender<TsMessage>, msg: TsMessage) {
    match msg.call_id() {
        None => {
            // Startup handshake — no call to route to. A send failure means
            // start() already moved on; harmless (the reader exits on EOF).
            let _ = ready_tx.send(msg);
        }
        Some(id) => {
            // Clone the Sender out under the lock, then send WITHOUT holding it
            // (a slow receiver must not block the reader / other routes).
            let route = routes.lock().unwrap().get(id).cloned();
            match route {
                // Send failure = that call already unwound; drop quietly.
                Some(tx) => {
                    let _ = tx.send(msg);
                }
                None => {
                    tracing::trace!("[functions] dropping message for unknown call_id {id}");
                }
            }
        }
    }
}

impl Drop for FnRunner {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---------------------------------------------------------------------------
// TraceBuilder helper (access user_id during execution)
// ---------------------------------------------------------------------------

impl TraceBuilder {
    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }
}

// ---------------------------------------------------------------------------
// DB operation executor
// ---------------------------------------------------------------------------

/// Execute a DB operation from a TypeScript function against the DataStore.
///
/// Returns the result value and optional row count (for traces).
///
/// Every read op also feeds the active dep recorder (see
/// [`crate::deps`]) so reactive query handlers get an automatic
/// dependency set. Writes are tracked too, even though reactive subs
/// only consume the read set — recording writes here is harmless and
/// keeps the entry points symmetric for any future "what does this
/// handler touch" analysis. The recorder is a thread-local no-op
/// when no reactive scope is active, so non-reactive paths pay nothing.
/// Map a wire-level [`DbOp`] to the coarse policy action used by
/// the gate. Returns `None` for ops the gate doesn't (yet) cover:
///   - QueryGraph: spans multiple entities, can't be checked with
///     a single-entity policy. Future work.
///   - Link/Unlink are updates of one foreign-key column, so they map
///     to Update; the gate sees the stored row and `{<fk>: target}`.
///   - Search/Paginate/List/Query/Lookup: all read-shaped, all
///     map to PolicyOp::Read.
///   - AdvisoryLock: not a data op, no policy meaning.
/// FK column that `relation` on `entity` sets, from the store's manifest.
fn relation_fk(store: &dyn DataStore, entity: &str, relation: Option<&str>) -> Option<String> {
    let relation = relation?;
    store
        .manifest()
        .entities
        .iter()
        .find(|e| e.name == entity)?
        .relations
        .iter()
        .find(|r| r.name == relation)
        .map(|r| r.field.clone())
}

fn policy_op_for(op: DbOp) -> Option<PolicyOp> {
    match op {
        DbOp::Get
        | DbOp::List
        | DbOp::Paginate
        | DbOp::Lookup
        | DbOp::Query
        | DbOp::Search
        | DbOp::VectorSearch => Some(PolicyOp::Read),
        DbOp::Insert => Some(PolicyOp::Insert),
        DbOp::Update | DbOp::Link | DbOp::Unlink => Some(PolicyOp::Update),
        DbOp::Delete => Some(PolicyOp::Delete),
        DbOp::QueryGraph | DbOp::AdvisoryLock => None,
    }
}

fn execute_db_op(
    store: &dyn DataStore,
    msg: &DbOpMessage,
    policy_gate: Option<&dyn PolicyGate>,
    auth: &AuthInfo,
    strict_policies: bool,
) -> (
    Result<serde_json::Value, pylon_http::DataError>,
    Option<usize>,
) {
    let strict_gate = policy_gate.filter(|_| {
        strict_policies && !msg.unsafe_op && !(auth.is_admin && auth.tenant_id.is_none())
    });
    let is_read = policy_op_for(msg.op) == Some(PolicyOp::Read) || msg.op == DbOp::QueryGraph;
    let read_gate = if msg.ssr_read && !msg.unsafe_op {
        policy_gate
    } else {
        strict_gate
    };
    let _read_scope = if is_read {
        read_gate.and_then(|gate| gate.begin_read())
    } else {
        None
    };
    if let Some(gate) = strict_gate {
        if let Some(op) = policy_op_for(msg.op) {
            if op == PolicyOp::Read {
                let access = if matches!(msg.op, DbOp::Search | DbOp::VectorSearch) {
                    gate.check_read_aggregate(&msg.entity, auth)
                } else {
                    gate.check_read_scan(&msg.entity, auth)
                };
                if let Err((code, message)) = access {
                    return (Err(pylon_http::DataError { code, message }), None);
                }
            } else {
                // Link/Unlink write the relation's FK column: `{fk: target}`
                // or `{fk: null}`. An unknown relation yields no patch; the
                // store call then fails with RELATION_NOT_FOUND.
                let link_patch = match msg.op {
                    DbOp::Link | DbOp::Unlink => {
                        relation_fk(store, &msg.entity, msg.relation.as_deref()).map(|fk| {
                            let target = match msg.op {
                                DbOp::Link => serde_json::Value::String(
                                    msg.target_id.clone().unwrap_or_default(),
                                ),
                                _ => serde_json::Value::Null,
                            };
                            serde_json::json!({ fk: target })
                        })
                    }
                    _ => None,
                };
                let data_for_check = match msg.op {
                    DbOp::Insert | DbOp::Update => msg.data.as_ref(),
                    DbOp::Link | DbOp::Unlink => link_patch.as_ref(),
                    _ => None,
                };
                // Update and Delete rules are row rules: they need the
                // stored row, and Update also checks the row after the
                // patch (see `PolicyEngine::check_entity_update`).
                let existing = match msg.op {
                    DbOp::Update | DbOp::Delete | DbOp::Link | DbOp::Unlink => {
                        match msg.id.as_deref() {
                            Some(id) => match store.get_by_id(&msg.entity, id) {
                                Ok(row) => row,
                                Err(e) => return (Err(e), None),
                            },
                            None => None,
                        }
                    }
                    _ => None,
                };
                if let Err((code, reason)) =
                    gate.check_op(op, &msg.entity, auth, existing.as_ref(), data_for_check)
                {
                    return (
                        Err(pylon_http::DataError {
                            code,
                            message: reason,
                        }),
                        None,
                    );
                }
            }
        }
    }
    let outcome = match msg.op {
        DbOp::Get => {
            let id = msg.id.as_deref().unwrap_or("");
            crate::deps::record_read(&msg.entity, Some(id));
            match store.get_by_id(&msg.entity, id) {
                Ok(Some(row)) => (Ok(row), Some(1)),
                Ok(None) => (Ok(serde_json::Value::Null), Some(0)),
                Err(e) => (Err(e), None),
            }
        }
        DbOp::List => {
            crate::deps::record_read(&msg.entity, None);
            match store.list(&msg.entity) {
                Ok(rows) => {
                    let count = rows.len();
                    (Ok(serde_json::json!(rows)), Some(count))
                }
                Err(e) => (Err(e), None),
            }
        }
        DbOp::Paginate => {
            crate::deps::record_read(&msg.entity, None);
            // Fetch limit+1 to detect "isDone" without an extra round trip,
            // matching the router's /api/entities/:e/cursor endpoint.
            let requested = msg.limit.unwrap_or(20).min(1000).max(1) as usize;
            let after = msg.after.as_deref();
            match store.list_after(&msg.entity, after, requested + 1) {
                Ok(mut rows) => {
                    let is_done = rows.len() <= requested;
                    if !is_done {
                        rows.truncate(requested);
                    }
                    let next_cursor = if is_done {
                        None
                    } else {
                        rows.last()
                            .and_then(|r| r.get("id"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    };
                    let count = rows.len();
                    (
                        Ok(serde_json::json!({
                            "page": rows,
                            "nextCursor": next_cursor,
                            "isDone": is_done,
                        })),
                        Some(count),
                    )
                }
                Err(e) => (Err(e), None),
            }
        }
        DbOp::Insert => {
            let data = msg.data.as_ref().cloned().unwrap_or(serde_json::json!({}));
            match store.insert(&msg.entity, &data) {
                Ok(id) => (Ok(serde_json::json!({"id": id})), None),
                Err(e) => (Err(e), None),
            }
        }
        DbOp::Update => {
            let id = msg.id.as_deref().unwrap_or("");
            let data = msg.data.as_ref().cloned().unwrap_or(serde_json::json!({}));
            match store.update(&msg.entity, id, &data) {
                Ok(updated) => (Ok(serde_json::json!({"updated": updated})), None),
                Err(e) => (Err(e), None),
            }
        }
        DbOp::Delete => {
            let id = msg.id.as_deref().unwrap_or("");
            match store.delete(&msg.entity, id) {
                Ok(deleted) => (Ok(serde_json::json!({"deleted": deleted})), None),
                Err(e) => (Err(e), None),
            }
        }
        DbOp::Lookup => {
            let field = msg.field.as_deref().unwrap_or("");
            let value = msg.value.as_deref().unwrap_or("");
            crate::deps::record_read(&msg.entity, None);
            match store.lookup(&msg.entity, field, value) {
                Ok(Some(row)) => (Ok(row), Some(1)),
                Ok(None) => (Ok(serde_json::Value::Null), Some(0)),
                Err(e) => (Err(e), None),
            }
        }
        DbOp::Query => {
            let filter = msg.data.as_ref().cloned().unwrap_or(serde_json::json!({}));
            crate::deps::record_read(&msg.entity, None);
            match store.query_filtered(&msg.entity, &filter) {
                Ok(rows) => {
                    let count = rows.len();
                    (Ok(serde_json::json!(rows)), Some(count))
                }
                Err(e) => (Err(e), None),
            }
        }
        DbOp::QueryGraph => {
            let query = msg.data.as_ref().cloned().unwrap_or(serde_json::json!({}));
            let graph_gate = if msg.ssr_read && !msg.unsafe_op {
                policy_gate
            } else {
                strict_gate
            };
            let result = (|| {
                if let Some(gate) = graph_gate {
                    check_graph_access(store.manifest(), &query, gate, auth, 0)?;
                }
                let mut result = store.query_graph(&query)?;
                if let Some(gate) = graph_gate {
                    filter_graph_result(
                        store.manifest(),
                        &query,
                        &mut result,
                        gate,
                        auth,
                        msg.ssr_read,
                    )?;
                }
                Ok(result)
            })();
            (result, None)
        }
        DbOp::Link => {
            let id = msg.id.as_deref().unwrap_or("");
            let relation = msg.relation.as_deref().unwrap_or("");
            let target_id = msg.target_id.as_deref().unwrap_or("");
            match store.link(&msg.entity, id, relation, target_id) {
                Ok(linked) => (Ok(serde_json::json!({"linked": linked})), None),
                Err(e) => (Err(e), None),
            }
        }
        DbOp::Unlink => {
            let id = msg.id.as_deref().unwrap_or("");
            let relation = msg.relation.as_deref().unwrap_or("");
            match store.unlink(&msg.entity, id, relation) {
                Ok(unlinked) => (Ok(serde_json::json!({"unlinked": unlinked})), None),
                Err(e) => (Err(e), None),
            }
        }
        DbOp::Search => {
            let query = msg.data.as_ref().cloned().unwrap_or(serde_json::json!({}));
            crate::deps::record_read(&msg.entity, None);
            match store.search(&msg.entity, &query) {
                Ok(result) => {
                    // Surface a coarse hit count for traces. The
                    // SearchResult JSON shape is `{ hits, ... }`; if
                    // the structure ever changes, the trace just
                    // shows None — never crashes.
                    let count = result
                        .get("hits")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len());
                    (Ok(result), count)
                }
                Err(e) => (Err(e), None),
            }
        }
        DbOp::VectorSearch => {
            let query = msg.data.as_ref().cloned().unwrap_or(serde_json::json!({}));
            crate::deps::record_read(&msg.entity, None);
            match store.vector_search(&msg.entity, &query) {
                Ok(result) => {
                    let count = result
                        .get("hits")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len());
                    (Ok(result), count)
                }
                Err(e) => (Err(e), None),
            }
        }
        DbOp::AdvisoryLock => {
            // The lock key rides on `entity`. SQLite path is a noop
            // (writers serialized); PG path issues
            // `pg_advisory_xact_lock` against the mutation tx.
            match store.advisory_lock(&msg.entity) {
                Ok(()) => (Ok(serde_json::json!({"locked": true})), None),
                Err(e) => (Err(e), None),
            }
        }
    };
    let outcome = if !msg.ssr_read {
        if let Some(gate) = strict_gate {
            filter_strict_read(msg, gate, auth, outcome)
        } else {
            outcome
        }
    } else {
        outcome
    };
    // SSR `serverData.*` reads are client-visible (serialized into
    // `__PYLON_DATA__`), so they get the entity/sync read treatment: per-row
    // policy filtering + `server_only`/`passwordHash` projection. `ctx.db.*`
    // (server-trust) and `serverData.unsafe.*` skip this.
    project_ssr_read(msg, policy_gate, auth, outcome)
}

fn policy_data_error((code, message): (String, String)) -> pylon_http::DataError {
    pylon_http::DataError { code, message }
}

fn invalid_graph(message: &str) -> pylon_http::DataError {
    pylon_http::DataError {
        code: "INVALID_QUERY".into(),
        message: message.into(),
    }
}

/// Check every entity in the query, including empty relation results.
fn check_graph_access(
    manifest: &pylon_kernel::AppManifest,
    query: &serde_json::Value,
    gate: &dyn PolicyGate,
    auth: &AuthInfo,
    depth: usize,
) -> Result<(), pylon_http::DataError> {
    if depth > 32 {
        return Err(invalid_graph("Graph includes exceed the depth limit"));
    }
    let entities = query
        .as_object()
        .ok_or_else(|| invalid_graph("Expected a graph query object"))?;
    for (name, options) in entities {
        let entity = manifest
            .entities
            .iter()
            .find(|e| e.name == *name)
            .ok_or_else(|| invalid_graph("Unknown graph entity"))?;
        gate.check_read_scan(name, auth)
            .map_err(policy_data_error)?;
        crate::deps::record_read(name, None);
        if let Some(includes) = options.get("include").and_then(|v| v.as_object()) {
            for (name, options) in includes {
                let relation = entity
                    .relations
                    .iter()
                    .find(|r| r.name == *name)
                    .ok_or_else(|| invalid_graph("Unknown graph relation"))?;
                check_graph_access(
                    manifest,
                    &serde_json::json!({&relation.target: options}),
                    gate,
                    auth,
                    depth + 1,
                )?;
            }
        }
    }
    Ok(())
}

fn filter_graph_result(
    manifest: &pylon_kernel::AppManifest,
    query: &serde_json::Value,
    result: &mut serde_json::Value,
    gate: &dyn PolicyGate,
    auth: &AuthInfo,
    client_visible: bool,
) -> Result<(), pylon_http::DataError> {
    let entries = result
        .as_object_mut()
        .ok_or_else(|| invalid_graph("Expected a graph result object"))?;
    for (entity, value) in entries {
        let options = query
            .get(entity)
            .ok_or_else(|| invalid_graph("Unexpected graph result entity"))?;
        filter_graph_rows(manifest, entity, options, value, gate, auth, client_visible)?;
    }
    Ok(())
}

/// Preserve relation cardinality while checking each returned row.
fn filter_graph_rows(
    manifest: &pylon_kernel::AppManifest,
    entity: &str,
    options: &serde_json::Value,
    value: &mut serde_json::Value,
    gate: &dyn PolicyGate,
    auth: &AuthInfo,
    client_visible: bool,
) -> Result<(), pylon_http::DataError> {
    let definition = manifest
        .entities
        .iter()
        .find(|e| e.name == entity)
        .ok_or_else(|| invalid_graph("Unknown graph result entity"))?;
    let is_many = value.is_array();
    let rows = match value.take() {
        serde_json::Value::Array(rows) => rows,
        row @ serde_json::Value::Object(_) => vec![row],
        serde_json::Value::Null => return Ok(()),
        _ => return Err(invalid_graph("Expected graph rows")),
    };
    let mut rows: Vec<_> = rows
        .into_iter()
        .filter(|row| {
            gate.check_op(PolicyOp::Read, entity, auth, None, Some(row))
                .is_ok()
        })
        .collect();
    for row in &mut rows {
        if let Some(includes) = options.get("include").and_then(|v| v.as_object()) {
            for (name, options) in includes {
                let relation = definition
                    .relations
                    .iter()
                    .find(|r| r.name == *name)
                    .ok_or_else(|| invalid_graph("Unknown graph relation"))?;
                if let Some(child) = row.get_mut(name) {
                    filter_graph_rows(
                        manifest,
                        &relation.target,
                        options,
                        child,
                        gate,
                        auth,
                        client_visible,
                    )?;
                }
            }
        }
    }
    if client_visible {
        rows = rows
            .into_iter()
            .filter_map(|mut row| {
                let mut related = serde_json::Map::new();
                if let Some(includes) = options.get("include").and_then(|value| value.as_object()) {
                    if let Some(object) = row.as_object_mut() {
                        for name in includes.keys() {
                            if let Some(child) = object.remove(name) {
                                related.insert(name.clone(), child);
                            }
                        }
                    }
                }
                let mut projected = gate
                    .filter_client_read(entity, auth, vec![row])
                    .into_iter()
                    .next()?;
                if let Some(object) = projected.as_object_mut() {
                    object.extend(related);
                }
                Some(projected)
            })
            .collect();
    }
    *value = if is_many {
        serde_json::Value::Array(rows)
    } else {
        rows.into_iter().next().unwrap_or(serde_json::Value::Null)
    };
    Ok(())
}

/// Strict function reads keep server fields but must obey row policies.
fn filter_strict_read(
    msg: &DbOpMessage,
    gate: &dyn PolicyGate,
    auth: &AuthInfo,
    outcome: (
        Result<serde_json::Value, pylon_http::DataError>,
        Option<usize>,
    ),
) -> (
    Result<serde_json::Value, pylon_http::DataError>,
    Option<usize>,
) {
    let (Ok(mut value), count) = outcome else {
        return outcome;
    };
    let check =
        |row: &serde_json::Value| gate.check_op(PolicyOp::Read, &msg.entity, auth, None, Some(row));
    match msg.op {
        DbOp::Get | DbOp::Lookup if !value.is_null() => {
            if let Err(error) = check(&value) {
                return (Err(policy_data_error(error)), None);
            }
        }
        DbOp::List | DbOp::Query | DbOp::Paginate | DbOp::Search | DbOp::VectorSearch => {
            let rows = match msg.op {
                DbOp::Paginate => value.get_mut("page"),
                DbOp::Search | DbOp::VectorSearch => value.get_mut("hits"),
                _ => Some(&mut value),
            };
            if let Some(rows) = rows.and_then(|v| v.as_array_mut()) {
                rows.retain(|row| {
                    let row = if msg.op == DbOp::VectorSearch {
                        &row["doc"]
                    } else {
                        row
                    };
                    check(row).is_ok()
                });
                let count = rows.len();
                return (Ok(value), Some(count));
            }
        }
        _ => {}
    }
    (Ok(value), count)
}

/// Apply the client-visible read fence (per-row policy filter + wire field
/// projection) to a read op's result when it came from SSR `serverData.*`
/// (safe, not `unsafe`). Non-read ops, `ctx.db` reads, unsafe reads, and error
/// results pass through untouched.
fn project_ssr_read(
    msg: &DbOpMessage,
    policy_gate: Option<&dyn PolicyGate>,
    auth: &AuthInfo,
    outcome: (
        Result<serde_json::Value, pylon_http::DataError>,
        Option<usize>,
    ),
) -> (
    Result<serde_json::Value, pylon_http::DataError>,
    Option<usize>,
) {
    if !msg.ssr_read || msg.unsafe_op {
        return outcome;
    }
    let Some(gate) = policy_gate else {
        return outcome;
    };
    let (Ok(value), _count) = outcome else {
        return outcome;
    };
    let entity = msg.entity.as_str();
    // Reshape each read op's result into a row list, run the client-read fence,
    // then reshape back to the op's response envelope.
    match msg.op {
        // Array-returning reads: filter + project the whole list.
        DbOp::List | DbOp::Query => {
            let rows = match value {
                serde_json::Value::Array(rows) => rows,
                other => return (Ok(other), None),
            };
            let filtered = gate.filter_client_read(entity, auth, rows);
            let count = filtered.len();
            (Ok(serde_json::Value::Array(filtered)), Some(count))
        }
        // Single-row reads: `null` when the row is filtered out.
        DbOp::Get | DbOp::Lookup => {
            if value.is_null() {
                return (Ok(value), Some(0));
            }
            let filtered = gate.filter_client_read(entity, auth, vec![value]);
            match filtered.into_iter().next() {
                Some(row) => (Ok(row), Some(1)),
                None => (Ok(serde_json::Value::Null), Some(0)),
            }
        }
        // Paginate envelope: `{ page: [...], nextCursor, isDone }` — fence the
        // page array, leave the cursor/isDone as-is.
        DbOp::Paginate => {
            let mut obj = match value {
                serde_json::Value::Object(obj) => obj,
                other => return (Ok(other), None),
            };
            if let Some(serde_json::Value::Array(rows)) = obj.remove("page") {
                let filtered = gate.filter_client_read(entity, auth, rows);
                let count = filtered.len();
                obj.insert("page".to_string(), serde_json::Value::Array(filtered));
                (Ok(serde_json::Value::Object(obj)), Some(count))
            } else {
                (Ok(serde_json::Value::Object(obj)), None)
            }
        }
        // Faceted search envelope: `{ hits, facetCounts, total }` — fence via
        // the dedicated search path (aggregate-safety gate + per-hit filter).
        DbOp::Search => match gate.filter_client_search(entity, auth, value) {
            Ok(filtered) => (Ok(filtered), None),
            Err((code, message)) => (Err(pylon_http::DataError { code, message }), None),
        },
        // Vector-search envelope: `{ hits: [{id, score, doc}], tookMs }` —
        // same aggregate-safety gate + per-hit doc fence, via the dedicated
        // vector path.
        DbOp::VectorSearch => match gate.filter_client_vector_search(entity, auth, value) {
            Ok(filtered) => (Ok(filtered), None),
            Err((code, message)) => (Err(pylon_http::DataError { code, message }), None),
        },
        // Graph results were checked in execute_db_op. Writes pass through.
        _ => (Ok(value), None),
    }
}

// ---------------------------------------------------------------------------
// Hydration bundle return type
// ---------------------------------------------------------------------------

/// Result returned by [`FnRunner::bundle_client`]. The host serves
/// any file under `outdir` at `/_pylon/build/<rel>` and reads
/// `manifest_path` to drive the per-route `<script>` +
/// `<link rel="modulepreload">` injection in the SSR head.
#[derive(Debug, Clone)]
pub struct BundleClientPaths {
    /// Absolute path to the manifest JSON.
    pub manifest_path: String,
    /// Absolute path to the bundle output directory.
    pub outdir: String,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FnCallError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for FnCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for FnCallError {}

/// Lift a `DataError` straight into a `FnCallError`. Lets the Postgres
/// mutation path use `PostgresDataStore::with_transaction(|store| ...)`
/// — its bound is `E: From<DataError>`, so any infrastructure failure
/// (lock poisoning, BEGIN/COMMIT) surfaces as a clean `FnCallError`
/// rather than needing manual mapping at the closure boundary. The
/// mapping is 1:1 because both error types carry just `{ code, message }`.
impl From<pylon_http::DataError> for FnCallError {
    fn from(e: pylon_http::DataError) -> Self {
        FnCallError {
            code: e.code,
            message: e.message,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_stream_that_just_finished_counts_as_activity() {
        let ops = super::LlmStreamOps::default();
        assert!(!ops.finished_within(std::time::Duration::from_secs(1)));
        {
            ops.active.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _guard = super::ActiveStreamGuard(
                std::sync::Arc::clone(&ops.active),
                std::sync::Arc::clone(&ops.last_finished_ms),
            );
        }
        assert!(!ops.any_active());
        assert!(ops.finished_within(std::time::Duration::from_secs(1)));
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(!ops.finished_within(std::time::Duration::from_millis(10)));
    }

    #[test]
    fn the_ready_wait_defaults_to_thirty_seconds() {
        // Only asserts the default; the env override is read the same way
        // as the other PYLON_FN_* settings.
        if std::env::var("PYLON_FN_READY_TIMEOUT_SECS").is_err() {
            assert_eq!(
                super::ready_timeout_from_env(),
                std::time::Duration::from_secs(30)
            );
        }
    }

    use super::*;
    use crate::protocol::{AuthInfo, DbOp, DbOpMessage};
    use pylon_http::DataError;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn fn_call_error_display() {
        let e = FnCallError {
            code: "TEST".into(),
            message: "fail".into(),
        };
        assert_eq!(format!("{e}"), "[TEST] fail");
    }

    #[test]
    fn per_function_timeout_overrides_global_and_feeds_supervisor() {
        use crate::registry::{FnAuthMode, FnDef};
        let def = |name: &str, t: Option<u64>| FnDef {
            name: name.into(),
            fn_type: FnType::Action,
            args_schema: None,
            internal: false,
            auth: FnAuthMode::User,
            timeout_secs: t,
        };
        let runner = FnRunner::new(8);
        runner.set_call_timeout(Duration::from_secs(30));
        // A long render, a normal fn, and a `0` (ignored — treated as unset).
        runner.record_fn_timeouts(&[
            def("renderSlideshowVariant", Some(300)),
            def("getBrand", None),
            def("noop", Some(0)),
        ]);

        // The declared-long function gets its own deadline; everything else
        // (declared 0, or absent from the map) falls back to the global 30s.
        assert_eq!(
            runner.deadline_for("renderSlideshowVariant"),
            Duration::from_secs(300)
        );
        assert_eq!(runner.deadline_for("getBrand"), Duration::from_secs(30));
        assert_eq!(runner.deadline_for("noop"), Duration::from_secs(30));
        assert_eq!(runner.deadline_for("unknownFn"), Duration::from_secs(30));

        // The supervisor reads the MAX so its wedge backstop never fires before
        // the longest legit call.
        assert_eq!(runner.max_fn_timeout_secs(), 300);

        // A redeploy with no long functions clears the override.
        runner.record_fn_timeouts(&[def("getBrand", None)]);
        assert_eq!(runner.max_fn_timeout_secs(), 0);
        assert_eq!(runner.deadline_for("getBrand"), Duration::from_secs(30));
    }

    #[test]
    fn route_message_delivers_only_to_the_matching_call() {
        use crate::protocol::{ReadyMessage, ReturnMessage};
        // The cross-request-leak guard: a message must reach ONLY the channel
        // registered under its own call_id.
        let routes: RouteTable = Mutex::new(HashMap::new());
        let (tx_a, rx_a) = mpsc::channel();
        let (tx_b, rx_b) = mpsc::channel();
        routes.lock().unwrap().insert("c_a".to_string(), tx_a);
        routes.lock().unwrap().insert("c_b".to_string(), tx_b);
        let (ready_tx, ready_rx) = mpsc::channel();

        let ret = |id: &str| {
            TsMessage::Return(ReturnMessage {
                call_id: id.to_string(),
                value: serde_json::json!(id),
            })
        };

        // A message for c_b reaches ONLY c_b — never c_a (the leak this guards).
        route_message(&routes, &ready_tx, ret("c_b"));
        assert!(
            rx_a.try_recv().is_err(),
            "call A must not receive B's message"
        );
        match rx_b.try_recv() {
            Ok(TsMessage::Return(m)) => assert_eq!(m.call_id, "c_b"),
            other => panic!("c_b's channel should have B's Return, got {other:?}"),
        }

        // The call_id-less Ready goes to the handshake channel, not any route.
        route_message(
            &routes,
            &ready_tx,
            TsMessage::Ready(ReadyMessage {
                functions: vec![],
                workflows: vec![],
                error: None,
            }),
        );
        assert!(matches!(ready_rx.try_recv(), Ok(TsMessage::Ready(_))));
        assert!(rx_a.try_recv().is_err());
        assert!(rx_b.try_recv().is_err());

        // A message for an UNKNOWN call_id (late frame after a call unwound) is
        // dropped — it must not leak into another live call or the handshake.
        route_message(&routes, &ready_tx, ret("c_ghost"));
        assert!(rx_a.try_recv().is_err());
        assert!(rx_b.try_recv().is_err());
        assert!(ready_rx.try_recv().is_err());
    }

    #[test]
    fn ts_message_call_id_is_exhaustive_and_only_ready_is_none() {
        use crate::protocol::{ReadyMessage, ReturnMessage};
        // Ready is the ONLY message without a call_id (the reader routes it to
        // the handshake). Any other variant returning None would be silently
        // dropped, so this locks the invariant the demux relies on.
        assert_eq!(
            TsMessage::Ready(ReadyMessage {
                functions: vec![],
                workflows: vec![],
                error: None,
            })
            .call_id(),
            None
        );
        assert_eq!(
            TsMessage::Return(ReturnMessage {
                call_id: "c_1".into(),
                value: serde_json::Value::Null,
            })
            .call_id(),
            Some("c_1")
        );
    }

    fn run_agent_req(user_id: &str, admin: bool) -> crate::protocol::RunAgentMessage {
        crate::protocol::RunAgentMessage {
            call_id: "c_1".into(),
            agent: "helper".into(),
            args: serde_json::json!({ "input": "hi" }),
            user_id: user_id.into(),
            admin,
            context: None,
        }
    }

    #[test]
    fn run_agent_needs_a_server_side_action_caller() {
        let req = run_agent_req("owner", false);
        // A public action a client can call, without elevation: refused.
        let err = run_agent_auth(&req, FnType::Action, false, false).unwrap_err();
        assert_eq!(err.0, "AGENT_RUN_FORBIDDEN");
        // Queries and mutations: refused whoever calls.
        for fn_type in [FnType::Query, FnType::Mutation] {
            let err = run_agent_auth(&req, fn_type, true, true).unwrap_err();
            assert_eq!(err.0, "AGENT_RUN_ACTIONS_ONLY");
        }
        // An internal action, or an admin (elevated) one: allowed.
        for (internal, admin) in [(true, false), (false, true)] {
            let auth = run_agent_auth(&req, FnType::Action, internal, admin).unwrap();
            assert_eq!(auth.user_id.as_deref(), Some("owner"));
            assert!(!auth.is_admin);
            assert!(!auth.is_guest);
        }
    }

    #[test]
    fn run_agent_admin_identity_needs_an_admin_caller() {
        let req = run_agent_req("owner", true);
        let err = run_agent_auth(&req, FnType::Action, true, false).unwrap_err();
        assert_eq!(err.0, "AGENT_RUN_FORBIDDEN");
        let auth = run_agent_auth(&req, FnType::Action, false, true).unwrap();
        assert!(auth.is_admin);
    }

    #[test]
    fn run_agent_needs_a_user() {
        let err =
            run_agent_auth(&run_agent_req("  ", false), FnType::Action, true, true).unwrap_err();
        assert_eq!(err.0, "INVALID_ARGS");
    }

    #[test]
    fn cancel_frame_shape_is_stable() {
        // A timeout now emits a `cancel` frame for the one offending call
        // instead of killing the multiplexed child (which failed every
        // co-tenant call and SSR render). The TS runtime keys off this
        // exact wire shape; runtime.ts's "cancel" dispatch arm must match.
        let msg = crate::protocol::CancelCallMessage::new("c_7".into(), "idle timeout");
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "cancel");
        assert_eq!(json["call_id"], "c_7");
        assert_eq!(json["reason"], "idle timeout");
    }

    #[test]
    fn lifetime_multiplier_bounds_a_chatty_runaway() {
        // Idle-timeout semantics mean activity extends the deadline; the
        // hard cap is what keeps "a stream chunk every second, forever"
        // from meaning forever.
        let timeout = Duration::from_secs(30);
        let cap = timeout.saturating_mul(MAX_CALL_LIFETIME_MULTIPLIER);
        assert_eq!(cap, Duration::from_secs(300));
    }

    // ---------------------------------------------------------------
    // Policy gate — Phase 2 of caller-aware ctx.db. The gate is
    // installed by the runtime crate; here we drive it with a stub
    // to lock in the wire-level semantics (env on/off, unsafe_op
    // bypass, admin bypass, op-kind mapping, denial wrapping).
    // ---------------------------------------------------------------

    /// In-memory store that always returns Ok-but-empty for reads
    /// and Ok-with-stub-id for writes. Fine for testing the gate —
    /// we only care whether the op reached the store or got
    /// short-circuited at the policy layer.
    struct AlwaysOkStore;
    impl pylon_http::DataStore for AlwaysOkStore {
        fn manifest(&self) -> &pylon_kernel::AppManifest {
            // `Project.org` → `orgId`, so the link gate test can resolve
            // the FK column.
            static M: std::sync::OnceLock<pylon_kernel::AppManifest> = std::sync::OnceLock::new();
            M.get_or_init(|| pylon_kernel::AppManifest {
                entities: vec![pylon_kernel::ManifestEntity {
                    name: "Project".into(),
                    relations: vec![pylon_kernel::ManifestRelation {
                        name: "org".into(),
                        target: "Org".into(),
                        field: "orgId".into(),
                        many: false,
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })
        }
        fn insert(&self, _: &str, _: &serde_json::Value) -> Result<String, DataError> {
            Ok("stub-id".into())
        }
        fn get_by_id(&self, _: &str, id: &str) -> Result<Option<serde_json::Value>, DataError> {
            // One stored row so write-gate tests can see what the gate got.
            if id == "stored-1" {
                return Ok(Some(
                    serde_json::json!({"id": "stored-1", "orgId": "org-a"}),
                ));
            }
            Ok(None)
        }
        fn list(&self, _: &str) -> Result<Vec<serde_json::Value>, DataError> {
            Ok(vec![])
        }
        fn list_after(
            &self,
            _: &str,
            _: Option<&str>,
            _: usize,
        ) -> Result<Vec<serde_json::Value>, DataError> {
            Ok(vec![])
        }
        fn update(&self, _: &str, _: &str, _: &serde_json::Value) -> Result<bool, DataError> {
            Ok(true)
        }
        fn delete(&self, _: &str, _: &str) -> Result<bool, DataError> {
            Ok(true)
        }
        fn lookup(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<Option<serde_json::Value>, DataError> {
            Ok(None)
        }
        fn link(&self, _: &str, _: &str, _: &str, _: &str) -> Result<bool, DataError> {
            Ok(true)
        }
        fn unlink(&self, _: &str, _: &str, _: &str) -> Result<bool, DataError> {
            Ok(true)
        }
        fn query_filtered(
            &self,
            _: &str,
            _: &serde_json::Value,
        ) -> Result<Vec<serde_json::Value>, DataError> {
            Ok(vec![])
        }
        fn query_graph(&self, _: &serde_json::Value) -> Result<serde_json::Value, DataError> {
            Ok(serde_json::json!({}))
        }
        fn transact(
            &self,
            _: &[serde_json::Value],
        ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
            Ok((true, vec![]))
        }
    }

    /// Stub gate that records every call and returns whatever the
    /// test set up. Records calls so assertions can verify the gate
    /// was reached (or wasn't) for each scenario.
    struct StubGate {
        calls: StdMutex<Vec<(PolicyOp, String, bool)>>,
        result: Result<(), (String, String)>,
    }
    impl PolicyGate for StubGate {
        fn check_op(
            &self,
            op: PolicyOp,
            entity: &str,
            auth: &AuthInfo,
            _existing: Option<&serde_json::Value>,
            _data: Option<&serde_json::Value>,
        ) -> Result<(), (String, String)> {
            self.calls
                .lock()
                .unwrap()
                .push((op, entity.to_string(), auth.is_admin));
            self.result.clone()
        }
    }

    fn allow_gate() -> StubGate {
        StubGate {
            calls: StdMutex::new(vec![]),
            result: Ok(()),
        }
    }

    fn deny_gate() -> StubGate {
        StubGate {
            calls: StdMutex::new(vec![]),
            result: Err(("POLICY_DENIED".to_string(), "stubbed deny".to_string())),
        }
    }

    fn user_auth() -> AuthInfo {
        AuthInfo {
            user_id: Some("u1".into()),
            is_admin: false,
            tenant_id: None,
            roles: vec![],
            is_guest: false,
        }
    }

    fn admin_auth() -> AuthInfo {
        AuthInfo {
            user_id: Some("admin".into()),
            is_admin: true,
            tenant_id: None,
            roles: vec![],
            is_guest: false,
        }
    }

    #[test]
    fn current_auth_snapshot_preserves_identity_tenant_roles_and_false_admin() {
        let auth = AuthInfo {
            user_id: Some("user-42".into()),
            is_admin: false,
            tenant_id: Some("tenant-7".into()),
            roles: vec!["editor".into(), "reviewer".into()],
            is_guest: false,
        };

        let snapshot = current_auth_snapshot(&auth, false);

        assert_eq!(snapshot.user_id, auth.user_id);
        assert_eq!(snapshot.tenant_id, auth.tenant_id);
        assert_eq!(snapshot.roles, vec!["editor", "reviewer"]);
        assert!(!snapshot.is_admin);
    }

    #[test]
    fn current_auth_snapshot_applies_elevated_admin_state() {
        let snapshot = current_auth_snapshot(&user_auth(), true);

        assert!(snapshot.is_admin);
    }

    #[test]
    fn current_auth_snapshot_preserves_true_admin_state() {
        let snapshot = current_auth_snapshot(&admin_auth(), true);

        assert!(snapshot.is_admin);
    }

    fn db_msg(op: DbOp, entity: &str, unsafe_op: bool) -> DbOpMessage {
        DbOpMessage {
            call_id: "c".into(),
            op_id: None,
            op,
            entity: entity.into(),
            id: Some("x".into()),
            data: None,
            field: None,
            value: None,
            relation: None,
            target_id: None,
            after: None,
            limit: None,
            unsafe_op,
            ssr_read: false,
        }
    }

    /// One gate call: the op, the stored row, and the payload.
    type GateCall = (
        PolicyOp,
        Option<serde_json::Value>,
        Option<serde_json::Value>,
    );

    /// Gate that records the stored row and the payload it was given.
    #[derive(Default)]
    struct RecordingGate {
        seen: StdMutex<Vec<GateCall>>,
    }
    impl PolicyGate for RecordingGate {
        fn check_op(
            &self,
            op: PolicyOp,
            _entity: &str,
            _auth: &AuthInfo,
            existing: Option<&serde_json::Value>,
            data: Option<&serde_json::Value>,
        ) -> Result<(), (String, String)> {
            self.seen
                .lock()
                .unwrap()
                .push((op, existing.cloned(), data.cloned()));
            Ok(())
        }
    }

    #[test]
    fn write_gate_gets_the_stored_row_and_the_patch() {
        // Update rules are checked against the stored row and the row after
        // the patch, so the gate needs both. Delete needs the stored row.
        let store = AlwaysOkStore;
        let gate = RecordingGate::default();
        let stored = serde_json::json!({"id": "stored-1", "orgId": "org-a"});
        let patch = serde_json::json!({"orgId": "org-b"});

        let mut update = db_msg(DbOp::Update, "Project", false);
        update.id = Some("stored-1".into());
        update.data = Some(patch.clone());
        let _ = execute_db_op(&store, &update, Some(&gate), &user_auth(), true);

        let mut delete = db_msg(DbOp::Delete, "Project", false);
        delete.id = Some("stored-1".into());
        let _ = execute_db_op(&store, &delete, Some(&gate), &user_auth(), true);

        let mut link = db_msg(DbOp::Link, "Project", false);
        link.id = Some("stored-1".into());
        link.relation = Some("org".into());
        link.target_id = Some("org-b".into());
        let _ = execute_db_op(&store, &link, Some(&gate), &user_auth(), true);

        let mut unlink = db_msg(DbOp::Unlink, "Project", false);
        unlink.id = Some("stored-1".into());
        unlink.relation = Some("org".into());
        let _ = execute_db_op(&store, &unlink, Some(&gate), &user_auth(), true);

        let seen = gate.seen.lock().unwrap();
        // Link/Unlink are updates of the FK column.
        assert_eq!(
            seen[2],
            (
                PolicyOp::Update,
                Some(stored.clone()),
                Some(serde_json::json!({"orgId": "org-b"}))
            )
        );
        assert_eq!(
            seen[3],
            (
                PolicyOp::Update,
                Some(stored.clone()),
                Some(serde_json::json!({"orgId": null}))
            )
        );
        assert_eq!(
            seen[0],
            (PolicyOp::Update, Some(stored.clone()), Some(patch))
        );
        assert_eq!(seen[1], (PolicyOp::Delete, Some(stored), None));
    }

    struct RowGate;
    impl PolicyGate for RowGate {
        fn check_read_scan(&self, entity: &str, _: &AuthInfo) -> Result<(), (String, String)> {
            if entity == "Org" {
                Err(("POLICY_DENIED".into(), "Org reads are forbidden".into()))
            } else {
                Ok(())
            }
        }

        fn check_op(
            &self,
            _: PolicyOp,
            _: &str,
            auth: &AuthInfo,
            _: Option<&serde_json::Value>,
            row: Option<&serde_json::Value>,
        ) -> Result<(), (String, String)> {
            if row.and_then(|r| r.get("owner")).and_then(|v| v.as_str()) == auth.user_id.as_deref()
            {
                Ok(())
            } else {
                Err(("POLICY_DENIED".into(), "Row belongs to another user".into()))
            }
        }

        fn filter_client_read(
            &self,
            entity: &str,
            auth: &AuthInfo,
            rows: Vec<serde_json::Value>,
        ) -> Vec<serde_json::Value> {
            FilterGate.filter_client_read(entity, auth, rows)
        }
    }

    #[test]
    fn strict_reads_filter_rows_without_removing_server_fields() {
        for op in [
            DbOp::List,
            DbOp::Query,
            DbOp::Paginate,
            DbOp::Search,
            DbOp::VectorSearch,
        ] {
            let value = match op {
                DbOp::Paginate => {
                    serde_json::json!({"page": two_rows(), "nextCursor": "2", "isDone": false})
                }
                DbOp::Search => serde_json::json!({"hits": two_rows()}),
                DbOp::VectorSearch => serde_json::json!({"hits": [
                    {"doc": two_rows()[0], "score": 0.9},
                    {"doc": two_rows()[1], "score": 0.8}
                ]}),
                _ => two_rows(),
            };
            let (result, count) = filter_strict_read(
                &db_msg(op, "Doc", false),
                &RowGate,
                &user_auth(),
                (Ok(value), Some(2)),
            );
            let result = result.unwrap();
            let rows = match op {
                DbOp::Paginate => &result["page"],
                DbOp::Search | DbOp::VectorSearch => &result["hits"],
                _ => &result,
            };
            assert_eq!(rows.as_array().unwrap().len(), 1, "{op:?}");
            let row = if op == DbOp::VectorSearch {
                &rows[0]["doc"]
            } else {
                &rows[0]
            };
            assert_eq!(row["secret"], "s1");
            assert_eq!(count, Some(1));
        }
        for op in [DbOp::Get, DbOp::Lookup] {
            let msg = db_msg(op, "Doc", false);
            let (result, _) = filter_strict_read(
                &msg,
                &RowGate,
                &user_auth(),
                (Ok(two_rows()[1].clone()), Some(1)),
            );
            assert_eq!(result.unwrap_err().code, "POLICY_DENIED");
            let (result, _) = filter_strict_read(
                &msg,
                &RowGate,
                &user_auth(),
                (Ok(two_rows()[0].clone()), Some(1)),
            );
            assert_eq!(result.unwrap()["secret"], "s1");
        }
    }

    #[test]
    fn graph_reads_filter_nested_relations_and_project_client_fields() {
        use pylon_kernel::{AppManifest, ManifestEntity, ManifestRelation};
        struct PublicFieldsGate;
        impl PolicyGate for PublicFieldsGate {
            fn check_op(
                &self,
                op: PolicyOp,
                entity: &str,
                auth: &AuthInfo,
                input: Option<&serde_json::Value>,
                row: Option<&serde_json::Value>,
            ) -> Result<(), (String, String)> {
                RowGate.check_op(op, entity, auth, input, row)
            }
            fn filter_client_read(
                &self,
                entity: &str,
                auth: &AuthInfo,
                rows: Vec<serde_json::Value>,
            ) -> Vec<serde_json::Value> {
                RowGate
                    .filter_client_read(entity, auth, rows)
                    .into_iter()
                    .map(|mut row| {
                        row.as_object_mut()
                            .unwrap()
                            .retain(|key, _| key == "id" || key == "owner");
                        row
                    })
                    .collect()
            }
        }
        let manifest = AppManifest {
            entities: vec![ManifestEntity {
                name: "Doc".into(),
                relations: vec![
                    ManifestRelation {
                        name: "children".into(),
                        target: "Doc".into(),
                        field: "id".into(),
                        many: true,
                    },
                    ManifestRelation {
                        name: "parent".into(),
                        target: "Doc".into(),
                        field: "parentId".into(),
                        many: false,
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let query = serde_json::json!({"Doc": {"include": {"children": {"include": {"parent": {}}}, "parent": {}}}});
        let raw = serde_json::json!({"Doc": [
            {"id": "1", "owner": "u1", "secret": "root-secret",
             "parent": {"id": "2", "owner": "u2", "secret": "foreign-secret"},
             "children": [
                 {"id": "3", "owner": "u1", "secret": "child-secret",
                  "parent": {"id": "4", "owner": "u2", "secret": "nested-secret"}},
                 {"id": "5", "owner": "u2", "secret": "foreign-child-secret"}
             ]},
            {"id": "6", "owner": "u2", "secret": "foreign-root-secret"}
        ]});
        check_graph_access(&manifest, &query, &RowGate, &user_auth(), 0).unwrap();
        for client_visible in [false, true] {
            let mut result = raw.clone();
            filter_graph_result(
                &manifest,
                &query,
                &mut result,
                &PublicFieldsGate,
                &user_auth(),
                client_visible,
            )
            .unwrap();
            let rows = result["Doc"].as_array().unwrap();
            assert_eq!(rows.len(), 1);
            assert!(rows[0]["parent"].is_null());
            let children = rows[0]["children"].as_array().unwrap();
            assert_eq!(children.len(), 1);
            assert!(children[0]["parent"].is_null());
            assert_eq!(rows[0].get("secret").is_none(), client_visible);
            assert_eq!(children[0].get("secret").is_none(), client_visible);
        }
    }

    #[test]
    fn graph_reads_check_included_entities_before_loading() {
        let mut manifest = pylon_http::DataStore::manifest(&AlwaysOkStore).clone();
        manifest.entities.push(pylon_kernel::ManifestEntity {
            name: "Org".into(),
            ..Default::default()
        });
        let query = serde_json::json!({"Project": {"include": {"org": {}}}});
        let error = check_graph_access(&manifest, &query, &RowGate, &user_auth(), 0).unwrap_err();
        assert_eq!(error.code, "POLICY_DENIED");
    }

    #[test]
    fn graph_trusted_reads_keep_the_explicit_bypass() {
        for (strict, unsafe_op, auth) in [
            (false, false, user_auth()),
            (true, true, user_auth()),
            (true, false, admin_auth()),
        ] {
            let gate = deny_gate();
            let mut msg = db_msg(DbOp::QueryGraph, "", unsafe_op);
            msg.data = Some(serde_json::json!({"Project": {}}));
            let (result, _) = execute_db_op(&AlwaysOkStore, &msg, Some(&gate), &auth, strict);
            assert!(result.is_ok());
            assert!(gate.calls.lock().unwrap().is_empty());
        }
    }

    struct ReadScopeGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl PolicyReadScope for ReadScopeGuard {}
    impl Drop for ReadScopeGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    struct ReadScopeGate {
        active: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        deny: bool,
    }
    impl PolicyGate for ReadScopeGate {
        fn begin_read(&self) -> Option<Box<dyn PolicyReadScope>> {
            self.active.fetch_add(1, Ordering::SeqCst);
            Some(Box::new(ReadScopeGuard(self.active.clone())))
        }
        fn check_op(
            &self,
            op: PolicyOp,
            _: &str,
            _: &AuthInfo,
            _: Option<&serde_json::Value>,
            _: Option<&serde_json::Value>,
        ) -> Result<(), (String, String)> {
            assert_eq!(
                self.active.load(Ordering::SeqCst),
                usize::from(op == PolicyOp::Read)
            );
            if self.deny {
                Err(("POLICY_DENIED".into(), "denied".into()))
            } else {
                Ok(())
            }
        }
        fn filter_client_read(
            &self,
            _: &str,
            _: &AuthInfo,
            rows: Vec<serde_json::Value>,
        ) -> Vec<serde_json::Value> {
            assert_eq!(self.active.load(Ordering::SeqCst), 1);
            rows
        }
    }

    #[test]
    fn read_scope_covers_checks_and_ends_on_success_or_denial() {
        for deny in [false, true] {
            let gate = ReadScopeGate {
                active: Default::default(),
                deny,
            };
            for op in [
                DbOp::Get,
                DbOp::List,
                DbOp::QueryGraph,
                DbOp::Insert,
                DbOp::Update,
            ] {
                let mut msg = db_msg(op, "Project", false);
                msg.id = Some("stored-1".into());
                if op == DbOp::QueryGraph {
                    msg.data = Some(serde_json::json!({"Project": {}}));
                }
                let (result, _) =
                    execute_db_op(&AlwaysOkStore, &msg, Some(&gate), &user_auth(), true);
                assert_eq!(result.is_err(), deny);
                assert_eq!(gate.active.load(Ordering::SeqCst), 0);
            }
        }
        let gate = ReadScopeGate {
            active: Default::default(),
            deny: false,
        };
        let mut msg = db_msg(DbOp::Get, "Project", false);
        msg.id = Some("stored-1".into());
        msg.ssr_read = true;
        assert!(
            execute_db_op(&AlwaysOkStore, &msg, Some(&gate), &user_auth(), false)
                .0
                .is_ok()
        );
        assert_eq!(gate.active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn strict_read_checks_the_returned_row() {
        let gate = RecordingGate::default();
        let mut msg = db_msg(DbOp::Get, "Project", false);
        msg.id = Some("stored-1".into());
        let (result, _) = execute_db_op(&AlwaysOkStore, &msg, Some(&gate), &user_auth(), true);
        let row = result.unwrap();
        assert!(row.is_object());
        assert!(gate
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|(op, _, data)| { *op == PolicyOp::Read && data.as_ref() == Some(&row) }));
    }

    #[test]
    fn strict_graph_read_cannot_skip_a_deny_policy() {
        let gate = deny_gate();
        let mut msg = db_msg(DbOp::QueryGraph, "", false);
        msg.data = Some(serde_json::json!({"Project": {}}));
        let (result, _) = execute_db_op(&AlwaysOkStore, &msg, Some(&gate), &user_auth(), true);
        assert_eq!(result.unwrap_err().code, "POLICY_DENIED");
    }

    #[test]
    fn strict_tenant_admin_cannot_skip_a_deny_policy() {
        let gate = deny_gate();
        let mut auth = admin_auth();
        auth.tenant_id = Some("tenant-a".into());
        let msg = db_msg(DbOp::Get, "Project", false);
        let (result, _) = execute_db_op(&AlwaysOkStore, &msg, Some(&gate), &auth, true);
        assert_eq!(result.unwrap_err().code, "POLICY_DENIED");
    }

    #[test]
    fn gate_skipped_when_strict_off() {
        // Default (strict_policies=false): gate never consulted.
        let store = AlwaysOkStore;
        let gate = allow_gate();
        let msg = db_msg(DbOp::Get, "Note", false);
        let (result, _) = execute_db_op(&store, &msg, Some(&gate), &user_auth(), false);
        assert!(result.is_ok());
        assert!(gate.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn gate_skipped_for_unsafe_op() {
        // Strict ON but the op is `unsafe_op: true` (came from
        // ctx.db.unsafe.*) — gate must skip.
        let store = AlwaysOkStore;
        let gate = allow_gate();
        let msg = db_msg(DbOp::Get, "Note", true);
        let (result, _) = execute_db_op(&store, &msg, Some(&gate), &user_auth(), true);
        assert!(result.is_ok());
        assert!(gate.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn gate_skipped_for_admin() {
        // Admin bypasses every policy gate by convention.
        let store = AlwaysOkStore;
        let gate = allow_gate();
        let msg = db_msg(DbOp::Get, "Note", false);
        let (result, _) = execute_db_op(&store, &msg, Some(&gate), &admin_auth(), true);
        assert!(result.is_ok());
        assert!(gate.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn gate_consulted_on_normal_read() {
        // Strict ON, not unsafe, not admin → gate fires.
        let store = AlwaysOkStore;
        let gate = allow_gate();
        let msg = db_msg(DbOp::Get, "Note", false);
        let (result, _) = execute_db_op(&store, &msg, Some(&gate), &user_auth(), true);
        assert!(result.is_ok());
        let calls = gate.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], (PolicyOp::Read, "Note".to_string(), false));
    }

    #[test]
    fn gate_denial_surfaces_as_data_error() {
        // The gate returning Err should short-circuit the op with
        // the supplied code + reason.
        let store = AlwaysOkStore;
        let gate = deny_gate();
        let msg = db_msg(DbOp::Get, "Note", false);
        let (result, count) = execute_db_op(&store, &msg, Some(&gate), &user_auth(), true);
        let err = result.expect_err("expected deny");
        assert_eq!(err.code, "POLICY_DENIED");
        assert!(err.message.contains("stubbed deny"));
        assert!(count.is_none());
    }

    // ---- SSR client-read fence (project_ssr_read wiring) ----

    /// Gate that stands in for the runtime adapter's client-read fence: keep
    /// only rows owned by the caller, and strip the `secret` field.
    struct FilterGate;
    impl PolicyGate for FilterGate {
        fn check_op(
            &self,
            _: PolicyOp,
            _: &str,
            _: &AuthInfo,
            _: Option<&serde_json::Value>,
            _: Option<&serde_json::Value>,
        ) -> Result<(), (String, String)> {
            Ok(())
        }
        fn filter_client_read(
            &self,
            _entity: &str,
            auth: &AuthInfo,
            rows: Vec<serde_json::Value>,
        ) -> Vec<serde_json::Value> {
            rows.into_iter()
                .filter(|r| r.get("owner").and_then(|v| v.as_str()) == auth.user_id.as_deref())
                .map(|mut r| {
                    if let Some(o) = r.as_object_mut() {
                        o.remove("secret");
                    }
                    r
                })
                .collect()
        }
        fn filter_client_search(
            &self,
            entity: &str,
            auth: &AuthInfo,
            mut result: serde_json::Value,
        ) -> Result<serde_json::Value, (String, String)> {
            if let Some(hits) = result.get_mut("hits").and_then(|v| v.as_array_mut()) {
                *hits = self.filter_client_read(entity, auth, std::mem::take(hits));
            }
            Ok(result)
        }
    }

    fn ssr_msg(op: DbOp, unsafe_op: bool, ssr_read: bool) -> DbOpMessage {
        let mut m = db_msg(op, "Doc", unsafe_op);
        m.ssr_read = ssr_read;
        m
    }

    fn two_rows() -> serde_json::Value {
        serde_json::json!([
            {"id": "1", "owner": "u1", "secret": "s1"},
            {"id": "2", "owner": "u2", "secret": "s2"},
        ])
    }

    #[test]
    fn ssr_read_list_is_filtered_and_projected() {
        // SSR serverData.list → per-row fence: only the caller's row survives,
        // and the `secret` field is stripped before it can reach the client.
        let (result, count) = project_ssr_read(
            &ssr_msg(DbOp::List, false, true),
            Some(&FilterGate),
            &user_auth(),
            (Ok(two_rows()), Some(2)),
        );
        assert_eq!(
            result.unwrap(),
            serde_json::json!([{"id": "1", "owner": "u1"}])
        );
        assert_eq!(count, Some(1));
    }

    #[test]
    fn ctx_db_read_is_not_fenced() {
        // ssr_read=false (server-function ctx.db) → server-trust, raw rows.
        let (result, _) = project_ssr_read(
            &ssr_msg(DbOp::List, false, false),
            Some(&FilterGate),
            &user_auth(),
            (Ok(two_rows()), Some(2)),
        );
        assert_eq!(result.unwrap(), two_rows());
    }

    #[test]
    fn ssr_unsafe_read_is_not_fenced() {
        // serverData.unsafe.* → explicit server-trust escape hatch, raw rows.
        let (result, _) = project_ssr_read(
            &ssr_msg(DbOp::List, true, true),
            Some(&FilterGate),
            &user_auth(),
            (Ok(two_rows()), Some(2)),
        );
        assert_eq!(result.unwrap(), two_rows());
    }

    #[test]
    fn ssr_get_of_forbidden_row_becomes_null() {
        // A single-row SSR read of a row the caller can't see → null.
        let (result, count) = project_ssr_read(
            &ssr_msg(DbOp::Get, false, true),
            Some(&FilterGate),
            &user_auth(),
            (
                Ok(serde_json::json!({"id": "2", "owner": "u2", "secret": "s2"})),
                Some(1),
            ),
        );
        assert_eq!(result.unwrap(), serde_json::Value::Null);
        assert_eq!(count, Some(0));
    }

    #[test]
    fn ssr_get_strips_fields_on_visible_row() {
        let (result, count) = project_ssr_read(
            &ssr_msg(DbOp::Get, false, true),
            Some(&FilterGate),
            &user_auth(),
            (
                Ok(serde_json::json!({"id": "1", "owner": "u1", "secret": "s1"})),
                Some(1),
            ),
        );
        assert_eq!(
            result.unwrap(),
            serde_json::json!({"id": "1", "owner": "u1"})
        );
        assert_eq!(count, Some(1));
    }

    #[test]
    fn ssr_search_hits_are_fenced() {
        let (result, _) = project_ssr_read(
            &ssr_msg(DbOp::Search, false, true),
            Some(&FilterGate),
            &user_auth(),
            (
                Ok(serde_json::json!({
                    "hits": [
                        {"id": "1", "owner": "u1", "secret": "s1"},
                        {"id": "2", "owner": "u2", "secret": "s2"},
                    ],
                    "total": 2,
                })),
                None,
            ),
        );
        assert_eq!(
            result.unwrap()["hits"],
            serde_json::json!([{"id": "1", "owner": "u1"}])
        );
    }

    // ---- NDJSON frame bound (memory-DoS fence) ----

    #[test]
    fn read_frame_drops_oversized_and_resyncs() {
        use std::io::Cursor;
        // A 100-byte frame with NO newline, then a normal frame. With a 10-byte
        // cap the first is dropped (Oversized) and the reader re-syncs on the
        // second — one hostile/huge frame can't grow the buffer without bound
        // or corrupt the frames after it.
        let mut input = vec![b'x'; 100];
        input.push(b'\n');
        input.extend_from_slice(b"{\"a\":1}\n");
        let mut r = Cursor::new(input);

        assert!(matches!(read_frame(&mut r, 10).unwrap(), Frame::Oversized));
        match read_frame(&mut r, 10).unwrap() {
            Frame::Data(buf) => assert_eq!(buf, b"{\"a\":1}\n"),
            _ => panic!("expected the next frame to parse cleanly after a drop"),
        }
        assert!(matches!(read_frame(&mut r, 10).unwrap(), Frame::Eof));
    }

    #[test]
    fn read_frame_passes_normal_frames() {
        use std::io::Cursor;
        let mut r = Cursor::new(b"{\"x\":1}\n".to_vec());
        match read_frame(&mut r, MAX_FRAME_BYTES).unwrap() {
            Frame::Data(buf) => assert_eq!(buf, b"{\"x\":1}\n"),
            _ => panic!("a within-bound frame must be returned as Data"),
        }
        assert!(matches!(
            read_frame(&mut r, MAX_FRAME_BYTES).unwrap(),
            Frame::Eof
        ));
    }

    #[test]
    fn op_mapping_matches_db_op_variants() {
        // Each DbOp maps to the right PolicyOp (or None for the
        // ops the gate doesn't cover).
        assert_eq!(policy_op_for(DbOp::Get), Some(PolicyOp::Read));
        assert_eq!(policy_op_for(DbOp::List), Some(PolicyOp::Read));
        assert_eq!(policy_op_for(DbOp::Paginate), Some(PolicyOp::Read));
        assert_eq!(policy_op_for(DbOp::Lookup), Some(PolicyOp::Read));
        assert_eq!(policy_op_for(DbOp::Query), Some(PolicyOp::Read));
        assert_eq!(policy_op_for(DbOp::Search), Some(PolicyOp::Read));
        assert_eq!(policy_op_for(DbOp::VectorSearch), Some(PolicyOp::Read));
        assert_eq!(policy_op_for(DbOp::Insert), Some(PolicyOp::Insert));
        assert_eq!(policy_op_for(DbOp::Update), Some(PolicyOp::Update));
        assert_eq!(policy_op_for(DbOp::Delete), Some(PolicyOp::Delete));
        // Unmapped ops — relations + advisory locks fall through
        // to the existing store call without a gate check.
        assert_eq!(policy_op_for(DbOp::QueryGraph), None);
        assert_eq!(policy_op_for(DbOp::Link), Some(PolicyOp::Update));
        assert_eq!(policy_op_for(DbOp::Unlink), Some(PolicyOp::Update));
        assert_eq!(policy_op_for(DbOp::AdvisoryLock), None);
    }

    #[test]
    fn ssr_serverdata_gate_is_read_only() {
        // The SSR render loop (render_route_inner) admits a `serverData` op
        // ONLY when `policy_op_for(op) == Some(PolicyOp::Read)` — a GET
        // render must never mutate. Every read op passes; every write op and
        // every uncheckable op (QueryGraph/Link/Unlink/AdvisoryLock) is
        // rejected with SSR_WRITE_FORBIDDEN. This asserts that exact gate so
        // a future DbOp addition can't silently open a write path to pages.
        let allowed = |op: DbOp| policy_op_for(op) == Some(PolicyOp::Read);
        for op in [
            DbOp::Get,
            DbOp::List,
            DbOp::Paginate,
            DbOp::Lookup,
            DbOp::Query,
            DbOp::Search,
            DbOp::VectorSearch,
        ] {
            assert!(allowed(op), "{op:?} must be allowed during SSR");
        }
        for op in [
            DbOp::Insert,
            DbOp::Update,
            DbOp::Delete,
            DbOp::QueryGraph,
            DbOp::Link,
            DbOp::Unlink,
            DbOp::AdvisoryLock,
        ] {
            assert!(!allowed(op), "{op:?} must be rejected during SSR");
        }
    }
}
