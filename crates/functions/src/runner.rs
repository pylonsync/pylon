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

use statecraft_http::DataStore;

use crate::protocol::*;
use crate::trace::{TraceBuilder, TraceLog};

/// Default ceiling on how long a single function call may take. Holds the
/// SQLite write lock for mutations, so this is also a backstop against a
/// runaway TS handler blocking the whole DB. Override via
/// [`FnRunner::set_call_timeout`] or `STATECRAFT_FN_CALL_TIMEOUT` (server-side).
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Stream callback — receives SSE chunks during execution
// ---------------------------------------------------------------------------

/// Callback invoked for each stream chunk during function execution.
/// The server layer converts these into SSE events on the HTTP response.
pub type StreamCallback = Box<dyn FnMut(&str) + Send>;

/// Callback invoked when a function calls `ctx.scheduler.runAfter/runAt`.
/// Returns `Ok(job_id)` on success or `Err(msg)` on persistence/queue
/// failure. The runner reports the error back to the calling handler so
/// users don't get a silent `{scheduled: true, id: ""}`.
pub type ScheduleHook = Box<
    dyn Fn(&str, serde_json::Value, Option<u64>, Option<u64>) -> Result<String, String>
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
    dyn Fn(
            &str,
            FnType,
            serde_json::Value,
            AuthInfo,
        ) -> Result<serde_json::Value, (String, String)>
        + Send
        + Sync,
>;

// ---------------------------------------------------------------------------
// Function runner
// ---------------------------------------------------------------------------

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
    /// Timeout for `recv()` between protocol messages. A handler that doesn't
    /// reply within this window is treated as stuck.
    call_timeout: Mutex<Duration>,
    /// The command and args that started the runtime. Stored so the supervisor
    /// can respawn on crash without the caller re-passing them.
    started_with: Mutex<Option<(String, Vec<String>)>>,
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
            call_timeout: Mutex::new(DEFAULT_CALL_TIMEOUT),
            started_with: Mutex::new(None),
        }
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
            .name("statecraft-fn-reader".into())
            .spawn(move || reader_loop(BufReader::new(stdout), tx))
            .map_err(|e| {
                kill_and_msg(&mut child, format!("Failed to spawn reader thread: {e}"))
            })?;

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
        *self.started_with.lock().unwrap() =
            Some((command.to_string(), args.iter().map(|s| s.to_string()).collect()));

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
        mut on_stream: Option<StreamCallback>,
        request: Option<crate::protocol::RequestInfo>,
    ) -> Result<(serde_json::Value, crate::trace::FnTrace), FnCallError> {

        let timeout = *self.call_timeout.lock().unwrap();
        let deadline = Instant::now() + timeout;

        let call_id = format!("c_{}", self.call_counter.fetch_add(1, Ordering::Relaxed));
        let mut trace = TraceBuilder::new(
            call_id.clone(),
            fn_name.to_string(),
            fn_type,
            auth.user_id.clone(),
        );

        // Send the call message. Attach HTTP request metadata when the
        // caller provided it — this lets TypeScript actions invoked via
        // /api/webhooks/:name see raw headers + body for signature checks.
        let mut call_msg = CallMessage::new(
            call_id.clone(),
            fn_name.to_string(),
            fn_type,
            args,
            auth,
        );
        if let Some(r) = request {
            call_msg = call_msg.with_request(r);
        }
        self.send(&call_msg)?;

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
                    let (result, row_count) =
                        execute_db_op(store, &db_msg);
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
                    let hook_result: Result<String, String> = {
                        let hook = self.schedule_hook.lock().unwrap();
                        match *hook {
                            Some(ref cb) => cb(
                                &sched.fn_name,
                                sched.args.clone(),
                                sched.delay_ms,
                                sched.run_at,
                            ),
                            None => Err("no schedule hook installed".into()),
                        }
                    };
                    let reply = match hook_result {
                        Ok(id) => DbResultMessage::ok(
                            call_id.clone(),
                            serde_json::json!({"scheduled": true, "id": id}),
                        ),
                        Err(e) => DbResultMessage::err(
                            call_id.clone(),
                            "SCHEDULE_FAILED",
                            &e,
                        ),
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

                TsMessage::RunFn(run) if run.call_id == call_id => {
                    // Nested function call (action calling query/mutation).
                    // Execute recursively. The nested call gets its own trace.
                    let nested_auth = AuthInfo {
                        user_id: trace.user_id().map(|s| s.to_string()),
                        is_admin: false,
                    };
                    // Prefer the nested_call_hook if installed — it lets the
                    // caller wrap mutations in their own BEGIN/COMMIT around
                    // a TxStore. Falling back to direct recursion leaves
                    // mutations non-transactional when triggered from an
                    // action (documented limitation).
                    let hook_result: Option<Result<serde_json::Value, (String, String)>> = {
                        let hook = self.nested_call_hook.lock().unwrap();
                        hook.as_ref().map(|cb| {
                            cb(&run.fn_name, run.fn_type, run.args.clone(), nested_auth.clone())
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
            Ok(0) => break,        // EOF — child exited
            Err(_) => break,       // pipe error — child gone
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
fn execute_db_op(
    store: &dyn DataStore,
    msg: &DbOpMessage,
) -> (Result<serde_json::Value, statecraft_http::DataError>, Option<usize>) {
    match msg.op {
        DbOp::Get => {
            let id = msg.id.as_deref().unwrap_or("");
            match store.get_by_id(&msg.entity, id) {
                Ok(Some(row)) => (Ok(row), Some(1)),
                Ok(None) => (Ok(serde_json::Value::Null), Some(0)),
                Err(e) => (Err(e), None),
            }
        }
        DbOp::List => match store.list(&msg.entity) {
            Ok(rows) => {
                let count = rows.len();
                (Ok(serde_json::json!(rows)), Some(count))
            }
            Err(e) => (Err(e), None),
        },
        DbOp::Paginate => {
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
            match store.lookup(&msg.entity, field, value) {
                Ok(Some(row)) => (Ok(row), Some(1)),
                Ok(None) => (Ok(serde_json::Value::Null), Some(0)),
                Err(e) => (Err(e), None),
            }
        }
        DbOp::Query => {
            let filter = msg.data.as_ref().cloned().unwrap_or(serde_json::json!({}));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fn_call_error_display() {
        let e = FnCallError {
            code: "TEST".into(),
            message: "fail".into(),
        };
        assert_eq!(format!("{e}"), "[TEST] fail");
    }
}
