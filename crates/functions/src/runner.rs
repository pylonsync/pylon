//! Function runner — executes TypeScript functions via the bidirectional protocol.
//!
//! The runner manages the connection to the Bun/Deno process and mediates
//! all communication. It handles DB operations, stream forwarding, scheduling,
//! and transaction management.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use pylon_http::DataStore;

use crate::protocol::*;
use crate::trace::{TraceBuilder, TraceLog};

/// Default ceiling on how long a single function call may take. Holds the
/// SQLite write lock for mutations, so this is also a backstop against a
/// runaway TS handler blocking the whole DB. Override via
/// [`FnRunner::set_call_timeout`] or `PYLON_FN_CALL_TIMEOUT` (server-side).
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Stream callback — receives SSE chunks during execution
// ---------------------------------------------------------------------------

/// Callback invoked for each stream chunk during function execution.
/// The server layer converts these into SSE events on the HTTP response.
pub type StreamCallback = Box<dyn FnMut(&str) + Send>;

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

/// Callback invoked when a running function asks to run *another* function
/// (action → query/mutation). The wrapper is responsible for any per-type
/// setup — notably wrapping mutations in their own BEGIN/COMMIT, which
/// can't happen inside `call_inner` because that path holds the io_lock
/// and is called with the outer action's non-transactional store.
///
/// Returns the nested function's return value or a `FnCallError`-shaped
/// `(code, message)` pair. The runner translates the error back into the
/// NDJSON protocol reply so the TS side sees the same shape it always did.
pub type NestedCallHook = Box<
    dyn Fn(&str, FnType, serde_json::Value, AuthInfo) -> Result<serde_json::Value, (String, String)>
        + Send
        + Sync,
>;

/// Callback invoked when an action calls `ctx.email.send(to, subject, body)`.
/// Returns Ok(()) on transport success, Err(reason) on failure.
///
/// The runner forwards this to the runtime's EmailAdapter (which knows
/// about PYLON_EMAIL_PROVIDER + credentials). Without this hook installed,
/// `ctx.email.send` returns a "transport not configured" error instead
/// of silently no-op'ing — apps shouldn't think email sent when it didn't.
pub type EmailHook = Box<dyn Fn(&str, &str, &str) -> Result<(), String> + Send + Sync>;

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
pub trait PolicyGate: Send + Sync {
    /// Decide whether `op` on `entity` is allowed for the
    /// caller described by `auth`. `data` carries the proposed
    /// row payload for writes (None for reads). Returns
    /// `Ok(())` to allow, or `Err((code, reason))` to deny.
    /// The runner surfaces the denial as a `DataError` with the
    /// supplied code; the TS handler sees it as a regular
    /// thrown error from `ctx.db.*`.
    fn check_op(
        &self,
        op: PolicyOp,
        entity: &str,
        auth: &crate::protocol::AuthInfo,
        data: Option<&serde_json::Value>,
    ) -> Result<(), (String, String)>;
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
    stdin: Mutex<Option<std::process::ChildStdin>>,
    /// Channel of parsed messages from the reader thread. Single consumer
    /// (callers serialize via `io_lock`), so no per-call demuxing.
    inbox: Mutex<Option<Receiver<TsMessage>>>,
    /// Held for the duration of a call to keep request/response in order.
    /// Also serializes the underlying single Bun process.
    io_lock: Mutex<()>,
    call_counter: AtomicU64,
    pub trace_log: TraceLog,
    schedule_hook: Mutex<Option<ScheduleHook>>,
    /// Optional override for nested function calls (action → query/mutation).
    /// When set, the runner delegates `RunFn` messages to this hook so the
    /// caller can wrap mutations in their own transaction. When absent, we
    /// fall back to the old recursive path (no transaction for nested
    /// mutations — documented limitation).
    nested_call_hook: Mutex<Option<NestedCallHook>>,
    /// Optional handler for `ctx.email.send(...)`. Apps that don't configure
    /// an email transport see `ctx.email.send` reject with an explicit
    /// error so silently-dropped invite emails surface in the action's
    /// error response.
    email_hook: Mutex<Option<EmailHook>>,
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
}

impl FnRunner {
    /// Create a new runner with the given trace log capacity.
    pub fn new(trace_capacity: usize) -> Self {
        Self {
            process: Mutex::new(None),
            stdin: Mutex::new(None),
            inbox: Mutex::new(None),
            io_lock: Mutex::new(()),
            call_counter: AtomicU64::new(0),
            trace_log: TraceLog::new(trace_capacity),
            schedule_hook: Mutex::new(None),
            nested_call_hook: Mutex::new(None),
            email_hook: Mutex::new(None),
            call_timeout: Mutex::new(DEFAULT_CALL_TIMEOUT),
            started_with: Mutex::new(None),
            policy_gate: Mutex::new(None),
        }
    }

    /// Install the caller-aware policy gate. The runtime crate
    /// supplies the adapter to `pylon_policy::PolicyEngine`; this
    /// crate stays free of that dep so the trait + indirection
    /// is the public seam.
    ///
    /// Only consulted when `PYLON_STRICT_FN_POLICIES=1` is set in
    /// the runner's environment. Without the env, the gate is a
    /// no-op even if installed — letting operators opt in per
    /// deploy without redeploying the binary.
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

    /// Install a callback used for nested function calls (action → query or
    /// mutation). The callback is responsible for transactional wrapping when
    /// the nested fn is a mutation. Without this hook, nested mutations share
    /// the outer action's non-transactional store and writes aren't atomic.
    pub fn set_nested_call_hook(&self, hook: NestedCallHook) {
        *self.nested_call_hook.lock().unwrap() = Some(hook);
    }

    /// Install a callback for `ctx.email.send(to, subject, body)` from
    /// action handlers. Wires through the runtime's configured EmailAdapter.
    /// When unset, `ctx.email.send` returns an explicit
    /// "EMAIL_TRANSPORT_NOT_CONFIGURED" error so authors see the gap
    /// instead of getting a silent no-op.
    pub fn set_email_hook(&self, hook: EmailHook) {
        *self.email_hook.lock().unwrap() = Some(hook);
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

        let (tx, rx): (Sender<TsMessage>, Receiver<TsMessage>) = mpsc::channel();
        std::thread::Builder::new()
            .name("pylon-fn-reader".into())
            .spawn(move || reader_loop(BufReader::new(stdout), tx))
            .map_err(|e| kill_and_msg(&mut child, format!("Failed to spawn reader thread: {e}")))?;

        // Read Ready BEFORE publishing the new IO. If we published first, a
        // concurrent caller could send a request and `recv()` would eat the
        // Ready in the catch-all match arm, leaving us in protocol limbo.
        let ready_msg = match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(m) => m,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("handshake timeout: TS runtime did not send Ready within 10s".into());
            }
        };
        let defs = match ready_msg {
            TsMessage::Ready(r) => {
                if let Some(err) = r.error {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("Runtime startup error: {err}"));
                }
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
        *self.inbox.lock().unwrap() = Some(rx);
        *self.process.lock().unwrap() = Some(child);
        *self.started_with.lock().unwrap() = Some((
            command.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        ));

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
                Ok(Some(_status)) => false, // exited
                Ok(None) => true,           // still running
                Err(_) => false,            // can't tell — assume dead
            },
        }
    }

    /// Deeper "is the runtime responsive?" probe — distinct from
    /// `is_alive` which only checks the OS process. This tries to
    /// acquire the io_lock (held for the duration of every active
    /// function call) within `timeout`. If we can grab it, no
    /// function is stuck holding it and the runtime is processing
    /// requests at the expected rate. If we can't, either the
    /// runtime is under sustained load OR a single function call is
    /// wedged — both are interesting signals for an external health
    /// probe.
    ///
    /// Used by /health/deep so Fly's health check fails when the bun
    /// runtime is thrashing, even though the HTTP listener itself is
    /// still up and answering /health 200. This is the failure mode
    /// that caused the runtime-kill cycle during today's incident:
    /// /health stayed green while every function call took 30s + got
    /// killed, taking all 150 functions offline during respawn.
    ///
    /// Returns Ok(()) when the runtime is responsive within timeout,
    /// Err(reason) when it isn't.
    pub fn health_probe(&self, timeout: Duration) -> Result<(), String> {
        if !self.is_alive() {
            return Err("runtime process not alive".into());
        }
        let deadline = Instant::now() + timeout;
        // Small busy-wait on try_lock. The std Mutex has no
        // try_lock_for so we poll every 10ms. The deadline bounds the
        // total cost at ~timeout — no risk of spinning forever.
        loop {
            if self.io_lock.try_lock().is_ok() {
                // Got it; runtime isn't holding the lock right now.
                // Lock immediately released as the guard drops.
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "io_lock contended for >{}ms — runtime may be wedged on a slow call",
                    timeout.as_millis()
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
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
        *self.inbox.lock().unwrap() = None;
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
    ) -> Result<(serde_json::Value, crate::trace::FnTrace), FnCallError> {
        // Serialize all top-level calls — one Bun process, NDJSON over stdio
        // is not multiplexed at this layer. Nested calls (action → query)
        // recurse through `call_inner` WITHOUT re-acquiring the lock.
        // `std::sync::Mutex` is not re-entrant, so doing otherwise wedges.
        let _io = self.io_lock.lock().unwrap();
        self.call_inner(store, fn_name, fn_type, args, auth, on_stream, request)
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
        caller_internal: bool,
    ) -> Result<(serde_json::Value, crate::trace::FnTrace), FnCallError> {
        let _io = self.io_lock.lock().unwrap();
        self.call_inner_with_caller_internal(
            store,
            fn_name,
            fn_type,
            args,
            auth,
            on_stream,
            request,
            caller_internal,
        )
    }

    /// Protocol-only call — assumes the caller already holds `io_lock`.
    /// This is the body of a `call()` minus the lock. It is `pub` so the
    /// nested-call hook in `FnOpsImpl` can re-enter the protocol for a
    /// transactional mutation wrap without re-acquiring the mutex (which
    /// would deadlock since `std::sync::Mutex` is not re-entrant).
    ///
    /// # Safety contract
    /// Do not call directly from code that didn't acquire `io_lock` via a
    /// prior `call()` invocation. Callers outside this crate should use
    /// `call()`; the only external caller is the nested-call hook.
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
            store, fn_name, fn_type, args, auth, on_stream, request, false,
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
        mut on_stream: Option<StreamCallback>,
        request: Option<crate::protocol::RequestInfo>,
        caller_internal: bool,
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
        let caller_tenant_id = auth.tenant_id.clone();
        let timeout = *self.call_timeout.lock().unwrap();
        let deadline = Instant::now() + timeout;

        let call_id = format!("c_{}", self.call_counter.fetch_add(1, Ordering::Relaxed));
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

        // Process messages until we get a return or error.
        loop {
            let msg = match self.recv(deadline) {
                Ok(m) => m,
                Err(e) if e.code == "FN_TIMEOUT" => {
                    // The child is now in an unknown state — it owns the call
                    // mid-flight and may be holding open whatever resource it
                    // had. Kill it; the supervisor will respawn. Better to
                    // lose the runtime than to wedge the SQLite write lock.
                    tracing::warn!(
                        "[functions] Killing TS runtime: call \"{}\" exceeded {:?}",
                        fn_name,
                        timeout
                    );
                    self.kill();
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
                    let per_op_auth = AuthInfo {
                        user_id: caller_user_id.clone(),
                        is_admin: caller_is_admin,
                        tenant_id: caller_tenant_id.clone(),
                        roles: gate_auth.roles.clone(),
                    };
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
                        cb(&chunk.data);
                    }
                }

                TsMessage::Schedule(sched) if sched.call_id == call_id => {
                    trace.record_schedule(&sched.fn_name, sched.delay_ms, sched.run_at);
                    let caller = ScheduleCallerInfo {
                        caller_internal,
                        caller_is_admin,
                        caller_user_id: caller_user_id.clone(),
                        caller_tenant_id: caller_tenant_id.clone(),
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
                    let reply = DbResultMessage::ok(
                        call_id.clone(),
                        serde_json::json!({"cancelled": true}),
                    );
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

                TsMessage::RunFn(run) if run.call_id == call_id => {
                    // Nested function call (action calling query/mutation).
                    // Execute recursively. The nested call gets its own trace
                    // but inherits user + tenant from the caller so row-level
                    // policies (`auth.tenantId == data.orgId`) keep working
                    // when an action stamps tenant-scoped writes via helper
                    // mutations. Callers that need to cross tenant boundaries
                    // must do so on the client side — no silent elevation
                    // happens here; the caller's tenant carries through.
                    let nested_auth = AuthInfo {
                        user_id: trace.user_id().map(|s| s.to_string()),
                        is_admin: false,
                        tenant_id: trace.tenant_id().map(|s| s.to_string()),
                        // Nested calls don't currently propagate the
                        // outer trace's roles — trace_log doesn't capture
                        // them. Empty here matches pre-roles behavior;
                        // RBAC-gated nested calls fall back to denying
                        // unless the outer is admin (which bypasses).
                        roles: Vec::new(),
                    };
                    // Prefer the nested_call_hook if installed — it lets the
                    // caller wrap mutations in their own BEGIN/COMMIT around
                    // a TxStore. Falling back to direct recursion leaves
                    // mutations non-transactional when triggered from an
                    // action (documented limitation).
                    let hook_result: Option<Result<serde_json::Value, (String, String)>> = {
                        let hook = self.nested_call_hook.lock().unwrap();
                        hook.as_ref().map(|cb| {
                            cb(
                                &run.fn_name,
                                run.fn_type,
                                run.args.clone(),
                                nested_auth.clone(),
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
                            // Already inside io_lock, so use call_inner. Nested
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
                    self.send(&reply)?;
                }

                TsMessage::SendEmail(req) if req.call_id == call_id => {
                    // Hand off to the runtime's email transport (configured
                    // via PYLON_EMAIL_PROVIDER). Without a hook installed
                    // we surface the missing-config gap explicitly so
                    // operators don't think their invite emails sent.
                    let result: Result<(), String> = {
                        let hook = self.email_hook.lock().unwrap();
                        match *hook {
                            Some(ref cb) => cb(&req.to, &req.subject, &req.body),
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
                    let fn_trace = trace.finish_ok(Some(ret.value.clone()));
                    self.trace_log.push(fn_trace.clone());
                    return Ok((ret.value, fn_trace));
                }

                TsMessage::Error(err) if err.call_id == call_id => {
                    let fn_trace = trace.finish_error(err.code.clone(), err.message.clone());
                    self.trace_log.push(fn_trace.clone());
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
        let mut stdin_guard = self.stdin.lock().unwrap();
        let stdin = stdin_guard.as_mut().ok_or_else(|| FnCallError {
            code: "RUNNER_NOT_STARTED".into(),
            message: "TypeScript function runner is not running".into(),
        })?;

        let mut line = serde_json::to_string(msg).map_err(|e| FnCallError {
            code: "SERIALIZE_FAILED".into(),
            message: format!("Failed to serialize message: {e}"),
        })?;
        line.push('\n');

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

    fn recv(&self, deadline: Instant) -> Result<TsMessage, FnCallError> {
        let inbox_guard = self.inbox.lock().unwrap();
        let inbox = inbox_guard.as_ref().ok_or_else(|| FnCallError {
            code: "RUNNER_NOT_STARTED".into(),
            message: "TypeScript function runner is not running".into(),
        })?;

        let now = Instant::now();
        let remaining = if deadline <= now {
            Duration::ZERO
        } else {
            deadline - now
        };

        match inbox.recv_timeout(remaining) {
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

/// Background reader thread: parses NDJSON lines from the Bun stdout into
/// TsMessage values and forwards them to the channel. Exits when stdout
/// closes (child died or was killed).
fn reader_loop(mut stdout: BufReader<std::process::ChildStdout>, tx: Sender<TsMessage>) {
    let mut line = String::new();
    loop {
        line.clear();
        match stdout.read_line(&mut line) {
            Ok(0) => break,  // EOF — child exited
            Err(_) => break, // pipe error — child gone
            Ok(_) => {}
        }
        match serde_json::from_str::<TsMessage>(line.trim()) {
            Ok(msg) => {
                if tx.send(msg).is_err() {
                    break; // Receiver dropped — runner shutting down
                }
            }
            Err(e) => {
                tracing::warn!(
                    "[functions] Skipping unparseable line from Bun runtime: {e} (line={:?})",
                    line.trim()
                );
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
///   - Link/Unlink: relation writes — the underlying row update
///     gets checked when the relation field is touched via
///     `ctx.db.update`. Skipping here avoids double-checks.
///   - Search/Paginate/List/Query/Lookup: all read-shaped, all
///     map to PolicyOp::Read.
///   - AdvisoryLock: not a data op, no policy meaning.
fn policy_op_for(op: DbOp) -> Option<PolicyOp> {
    match op {
        DbOp::Get | DbOp::List | DbOp::Paginate | DbOp::Lookup | DbOp::Query | DbOp::Search => {
            Some(PolicyOp::Read)
        }
        DbOp::Insert => Some(PolicyOp::Insert),
        DbOp::Update => Some(PolicyOp::Update),
        DbOp::Delete => Some(PolicyOp::Delete),
        DbOp::QueryGraph | DbOp::Link | DbOp::Unlink | DbOp::AdvisoryLock => None,
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
    // Caller-aware policy gate. Three guards before consulting:
    //   1. Gate must be installed (runtime wires the adapter at
    //      startup; older embeddings / wasm leave it None).
    //   2. Strict mode must be on (PYLON_STRICT_FN_POLICIES=1).
    //   3. The op must NOT carry unsafe_op — `ctx.db.unsafe.*`
    //      explicitly opts out of the gate.
    //
    // Admin callers bypass too — same convention as policy
    // engine + function-level auth gate. Ops scripts + the
    // `auth.elevate({ admin: true })` path inside webhooks
    // need the bypass to work everywhere without wildcard
    // policy expressions on every entity.
    if let Some(gate) = policy_gate {
        if strict_policies && !msg.unsafe_op && !auth.is_admin {
            if let Some(op) = policy_op_for(msg.op) {
                let data_for_check = match msg.op {
                    DbOp::Insert | DbOp::Update => msg.data.as_ref(),
                    _ => None,
                };
                if let Err((code, reason)) = gate.check_op(op, &msg.entity, auth, data_for_check) {
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
    match msg.op {
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
            match store.query_graph(&query) {
                Ok(result) => (Ok(result), None),
                Err(e) => (Err(e), None),
            }
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
        DbOp::AdvisoryLock => {
            // The lock key rides on `entity`. SQLite path is a noop
            // (writers serialized); PG path issues
            // `pg_advisory_xact_lock` against the mutation tx.
            match store.advisory_lock(&msg.entity) {
                Ok(()) => (Ok(serde_json::json!({"locked": true})), None),
                Err(e) => (Err(e), None),
            }
        }
    }
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
            // The gate is store-agnostic; we never reach the manifest
            // from execute_db_op. Returning a leaked default keeps
            // the impl trivial.
            static M: std::sync::OnceLock<pylon_kernel::AppManifest> = std::sync::OnceLock::new();
            M.get_or_init(pylon_kernel::AppManifest::default)
        }
        fn insert(&self, _: &str, _: &serde_json::Value) -> Result<String, DataError> {
            Ok("stub-id".into())
        }
        fn get_by_id(&self, _: &str, _: &str) -> Result<Option<serde_json::Value>, DataError> {
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
        }
    }

    fn admin_auth() -> AuthInfo {
        AuthInfo {
            user_id: Some("admin".into()),
            is_admin: true,
            tenant_id: None,
            roles: vec![],
        }
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
        }
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
        assert_eq!(policy_op_for(DbOp::Insert), Some(PolicyOp::Insert));
        assert_eq!(policy_op_for(DbOp::Update), Some(PolicyOp::Update));
        assert_eq!(policy_op_for(DbOp::Delete), Some(PolicyOp::Delete));
        // Unmapped ops — relations + advisory locks fall through
        // to the existing store call without a gate check.
        assert_eq!(policy_op_for(DbOp::QueryGraph), None);
        assert_eq!(policy_op_for(DbOp::Link), None);
        assert_eq!(policy_op_for(DbOp::Unlink), None);
        assert_eq!(policy_op_for(DbOp::AdvisoryLock), None);
    }
}
