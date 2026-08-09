/**
 * Function runtime — the Bun process that loads and executes TypeScript functions.
 *
 * Protocol: NDJSON over stdin/stdout.
 *
 * Usage:
 *   bun run packages/functions/src/runtime.ts ./functions
 *
 * Design:
 * - A single reader consumes lines from stdin and dispatches by message type.
 * - Incoming `call` messages launch a handler.
 * - Incoming `result` messages resolve a pending RPC keyed by call_id.
 * - Each call's handler has at most ONE outstanding RPC at a time (it awaits
 *   each ctx.db / ctx.scheduler / ctx.runMutation call), so the map never
 *   needs to queue multiple RPCs per call_id.
 */

import type {
  DbReader,
  DbWriter,
  EmailAttachment,
  EmailOptions,
  EmailSender,
  Files,
  Stream,
  Scheduler,
  Llm,
  LlmCompleteRequest,
  LlmCompleteResponse,
  LlmStreamEvent,
  Rooms,
  Connections,
  QueryCtx,
  MutationCtx,
  ActionCtx,
  FnDefinition,
  AuthInfo,
} from "./types";
import { normalizeAuthClaims } from "./auth";
import { makeRequireMember } from "./member";
import { isDevMode } from "./ssr-runtime";
import { validateArgs } from "./validators";
import { readdirSync } from "fs";
import { join, basename } from "path";

// Bun runtime globals this process uses. The runtime executes under Bun, but
// consuming apps type-check this source under node/DOM where the `Bun` global
// is absent — so declare the surface we touch (mirrors the ambient in
// ssr-client-bundler.ts). Keeps `tsc` clean in a scaffolded app.
interface BunFileSink {
  write(chunk: string): number;
  flush(): number | Promise<number>;
}
declare const Bun: {
  write(destination: unknown, input: string): Promise<number>;
  stdout: { writer(): BunFileSink };
  stderr: unknown;
  stdin: { stream(): ReadableStream<Uint8Array> };
};

// ---------------------------------------------------------------------------
// Protocol types
// ---------------------------------------------------------------------------

interface CallMessage {
  type: "call";
  call_id: string;
  fn_name: string;
  fn_type: "query" | "mutation" | "action";
  args: Record<string, unknown>;
  auth: AuthInfo;
}

interface ResultMessage {
  type: "result";
  call_id: string;
  data?: unknown;
  error?: { code: string; message: string };
}

// ---------------------------------------------------------------------------
// Send
// ---------------------------------------------------------------------------

// Protocol frames go through ONE FileSink over stdout. The old
// `Bun.write(Bun.stdout, line)` returned an UNAWAITED promise, so once a
// runner multiplexes concurrent calls (since the v0.3.259 runner change) two
// handlers emitting frames larger than PIPE_BUF (~4KB) could race their
// write() syscalls and interleave on stdout — the host's NDJSON reader then
// failed to parse the corrupted line and dropped the frame, hanging the caller
// to its timeout. A single sink appends every frame to one ordered buffer, so
// writes can't interleave; `send` is the only stdout writer.
let stdoutSink: BunFileSink | undefined;
function send(msg: Record<string, unknown>): void {
  const line = JSON.stringify(msg) + "\n";
  if (!stdoutSink) stdoutSink = Bun.stdout.writer();
  stdoutSink.write(line);
  // flush() can return a Promise under pipe backpressure. The single sink
  // already preserves frame order, so we don't await — just keep a flush
  // error (e.g. EPIPE after the host exits) from becoming an unhandled
  // rejection.
  const flushed = stdoutSink.flush();
  if (flushed && typeof (flushed as Promise<number>).then === "function") {
    (flushed as Promise<number>).then(undefined, () => {});
  }
}

/**
 * Redirect console.* from user code to stderr so handlers can't accidentally
 * emit a line that looks like a protocol frame and confuse the Rust reader.
 *
 * Before this guard, a handler calling `console.log('{"type":"return",...}')`
 * — either intentionally or by logging an object shaped that way — would be
 * parsed by the host as a real protocol message. Moving all console output
 * to stderr keeps stdout reserved for NDJSON protocol frames only.
 *
 * The original console methods are saved on the console object as
 * `__stdoutLog` etc. in case the runtime itself needs to write diagnostics
 * to stdout for some reason (it currently doesn't).
 */
function fenceStdout(): void {
  const toStderr = (prefix: string) => (...args: unknown[]) => {
    const line = args
      .map((a) => {
        if (typeof a === "string") return a;
        // Error: JSON.stringify yields `{}` because message/stack are
        // non-enumerable. That made `console.error("x:", err)` log as `x: {}`,
        // hiding the real failure from operators. Unwrap by hand.
        if (a instanceof Error) {
          const parts = [a.stack || `${a.name}: ${a.message}`];
          const code = (a as { code?: unknown }).code;
          if (code !== undefined) parts.push(`code=${String(code)}`);
          const cause = (a as { cause?: unknown }).cause;
          if (cause !== undefined) {
            try {
              parts.push(`cause=${cause instanceof Error ? cause.stack || cause.message : JSON.stringify(cause)}`);
            } catch {
              parts.push(`cause=${String(cause)}`);
            }
          }
          return parts.join(" ");
        }
        try {
          return JSON.stringify(a);
        } catch {
          return String(a);
        }
      })
      .join(" ");
    Bun.write(Bun.stderr, `${prefix}${line}\n`);
  };
  // Intentional: we want console.* for user handlers to go to stderr.
  // Overwrite the globals before any user code is loaded.
  const c = globalThis.console as unknown as Record<string, unknown>;
  c.__stdoutLog = c.log;
  c.log = toStderr("");
  c.info = toStderr("");
  c.warn = toStderr("[warn] ");
  c.error = toStderr("[error] ");
  c.debug = toStderr("[debug] ");
}

// ---------------------------------------------------------------------------
// Single reader + dispatcher
// ---------------------------------------------------------------------------

/**
 * Pending RPCs keyed by op_id (with a fallback to call_id for legacy hosts
 * that don't echo op_id). Each in-flight host → TS RPC gets its own
 * op_id so two concurrent DB ops from the same handler —
 * `Promise.all([ctx.db.get(a), ctx.db.get(b)])` — don't collide on the
 * outer call_id. Scheduler/runFn replies still route by call_id (one
 * outstanding per call is correct for those).
 */
const pendingRpcs = new Map<
  string,
  {
    resolve: (data: unknown) => void;
    reject: (err: Error) => void;
    timeout: ReturnType<typeof setTimeout>;
  }
>();

/**
 * Event sinks for in-flight streaming RPCs, keyed by op_id. Separate
 * from `pendingRpcs` because these fire many times and never settle
 * the promise — the terminal `result` message does that.
 */
const streamSinks = new Map<string, (event: unknown) => void>();

let opSeq = 0;
function nextOpId(callId: string): string {
  opSeq += 1;
  return `${callId}#${opSeq}`;
}

/**
 * Upper bound on how long an individual host → TS RPC (e.g. `ctx.db.get`)
 * can wait for a reply. The Rust side enforces its own per-handler timeout
 * (PYLON_FN_CALL_TIMEOUT, default 30s), but if a protocol frame gets
 * truncated or dropped, the awaiting promise would hang forever. This is
 * the safety net.
 *
 * 60s is deliberately longer than the Rust-side call timeout so that the
 * host always gets to time out first (with a meaningful error), not the
 * TS side (with a generic orphaned-rpc error).
 */
const RPC_TIMEOUT_MS = 60_000;

async function readerLoop(): Promise<void> {
  const reader = Bun.stdin.stream().getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() || "";

    for (const line of lines) {
      if (!line.trim()) continue;
      dispatch(line);
    }
  }

  if (buffer.trim()) dispatch(buffer);

  // stdin closed — the host is gone. Reject every pending RPC so awaiting
  // handlers unwind instead of hanging and keeping the Bun process alive
  // forever. Clearing timers avoids keeping the event loop ticking either.
  for (const [callId, pending] of pendingRpcs) {
    clearTimeout(pending.timeout);
    pending.reject(
      new Error(`host disconnected before reply (call_id=${callId})`),
    );
  }
  pendingRpcs.clear();
}

function dispatch(line: string): void {
  let msg: { type: string } & Record<string, unknown>;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }

  if (msg.type === "call") {
    // Launch handler; errors are reported back via the protocol, not thrown.
    handleCall(msg as unknown as CallMessage).catch((err) => {
      send({
        type: "error",
        call_id: (msg as unknown as CallMessage).call_id,
        code: "HANDLER_CRASH",
        message: err?.message || String(err),
      });
    });
  } else if (msg.type === "render_route") {
    // SSR dispatch — file-based page render. Lazy-imported so
    // projects without SSR routes don't pay the react-dom cost on
    // startup. handleRenderRoute manages its own error frames; we
    // still catch here so a bare throw can't kill the runtime.
    import("./ssr-runtime")
      .then((mod) =>
        mod.handleRenderRoute(
          msg as unknown as Parameters<typeof mod.handleRenderRoute>[0],
          send,
        ),
      )
      .catch((err) => {
        const devMode =
          process.env.PYLON_DEV_MODE === "1" ||
          process.env.PYLON_DEV_MODE === "true";
        send({
          type: "error",
          call_id: (msg as unknown as { call_id: string }).call_id,
          code: "SSR_RUNTIME_CRASH",
          // Dev: full stack for the host's error overlay. Prod: message only.
          message:
            (devMode && err?.stack ? String(err.stack) : err?.message) ||
            String(err),
        });
      });
  } else if (msg.type === "handle_form") {
    // route.ts form/method handler (#276) — lazy-imported like SSR so
    // projects without route handlers pay nothing on startup.
    import("./ssr-form-runtime")
      .then((mod) =>
        mod.handleForm(
          msg as unknown as Parameters<typeof mod.handleForm>[0],
          send,
        ),
      )
      .catch((err) => {
        send({
          type: "error",
          call_id: (msg as unknown as { call_id: string }).call_id,
          code: "SSR_FORM_RUNTIME_CRASH",
          message: err?.message || String(err),
        });
      });
  } else if (msg.type === "bundle_client") {
    // Hydration — build the client-side bundle once and report
    // the path back. Lazy-imported for the same reason as SSR.
    import("./ssr-client-bundler")
      .then((mod) =>
        mod.handleBundleClient(
          msg as unknown as Parameters<typeof mod.handleBundleClient>[0],
          send,
        ),
      )
      .catch((err) => {
        send({
          type: "bundle_client_result",
          call_id: (msg as unknown as { call_id: string }).call_id,
          path: "",
          error: err?.message || String(err),
        });
      });
  } else if (msg.type === "llm_event") {
    const ev = msg as unknown as {
      call_id: string;
      op_id?: string;
      event: unknown;
    };
    const sink = streamSinks.get(ev.op_id ?? ev.call_id);
    if (sink) sink(ev.event);
  } else if (msg.type === "result") {
    const res = msg as unknown as ResultMessage & { op_id?: string };
    // Prefer op_id when the host sent it. Fall back to call_id for replies
    // that don't have one (scheduler / runFn) and for legacy hosts.
    const key = res.op_id ?? res.call_id;
    const pending = pendingRpcs.get(key);
    if (!pending) return;
    pendingRpcs.delete(key);
    clearTimeout(pending.timeout);
    if (res.error) {
      const err = new Error(res.error.message);
      (err as any).code = res.error.code;
      pending.reject(err);
    } else {
      pending.resolve(res.data);
    }
  }
}

/**
 * RPC for DB operations: mints a per-op id so two concurrent DB ops from
 * the same handler can be in flight at once without colliding. The host
 * echoes `op_id` back in the `result` reply, which the dispatcher uses
 * to route the resolution.
 */
function rpcDb(
  callId: string,
  msg: Record<string, unknown>,
): Promise<unknown> {
  const opId = nextOpId(callId);
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      if (pendingRpcs.has(opId)) {
        pendingRpcs.delete(opId);
        reject(
          new Error(
            `RPC timed out after ${RPC_TIMEOUT_MS}ms (call_id=${callId} op_id=${opId})`,
          ),
        );
      }
    }, RPC_TIMEOUT_MS);
    pendingRpcs.set(opId, { resolve, reject, timeout });
    send({ ...msg, call_id: callId, op_id: opId });
  });
}

/**
 * RPC that receives interim events before its terminal reply
 * (`ctx.llm.stream`). Mints an op_id like {@link rpcDb}, and also
 * registers an event sink the dispatcher routes `llm_event` messages
 * to. The promise resolves only on the final `result`.
 *
 * Each event RESTARTS the idle timeout: a stream that keeps producing
 * is alive, however long it runs in total, while one whose provider
 * goes silent still trips the safety net. The host's own call deadline
 * remains the real upper bound.
 */
function rpcStreaming(
  callId: string,
  msg: Record<string, unknown>,
  onEvent: (event: unknown) => void,
): Promise<unknown> {
  const opId = nextOpId(callId);
  return new Promise((resolve, reject) => {
    const fail = () => {
      if (pendingRpcs.has(opId)) {
        pendingRpcs.delete(opId);
        streamSinks.delete(opId);
        reject(
          new Error(
            `RPC timed out after ${RPC_TIMEOUT_MS}ms with no stream activity (call_id=${callId} op_id=${opId})`,
          ),
        );
      }
    };
    const entry = {
      resolve: (data: unknown) => {
        streamSinks.delete(opId);
        resolve(data);
      },
      reject: (err: Error) => {
        streamSinks.delete(opId);
        reject(err);
      },
      timeout: setTimeout(fail, RPC_TIMEOUT_MS),
    };
    pendingRpcs.set(opId, entry);
    streamSinks.set(opId, (event) => {
      clearTimeout(entry.timeout);
      entry.timeout = setTimeout(fail, RPC_TIMEOUT_MS);
      // A throwing sink must not abandon the pending RPC — the host is
      // still going to send a terminal result, and swallowing here
      // keeps the handler's own error the one that surfaces.
      try {
        onEvent(event);
      } catch {
        /* handler-side callback error; the stream continues */
      }
    });
    send({ ...msg, call_id: callId, op_id: opId });
  });
}

/**
 * Tail of the in-order queue of legacy (call_id-keyed) RPCs, per call.
 * See {@link rpc} for why these serialize.
 */
const rpcQueues = new Map<string, Promise<unknown>>();

/**
 * RPC for non-db protocol replies (scheduler.runAfter, elevate, email,
 * nested function calls, etc.). These reply frames carry no op_id, so
 * only one can be in flight per call_id — but instead of rejecting a
 * second request (the old "concurrent RPC attempted on same call_id"
 * error), requests QUEUE: each one waits for the previous request on
 * the same call_id to settle, then sends. The host drains a per-call
 * channel strictly in order, so wire order == call order and the
 * replies can't cross.
 *
 * This is what makes an un-awaited `ctx.auth.elevate(...)` followed by
 * `ctx.scheduler.runAfter(...)` behave correctly: the elevate frame is
 * sent (and replied to) before the schedule frame goes out, so the
 * host-side admin flag is already set when the schedule arm reads it.
 * `Promise.all` over these ctx calls serializes for the same reason.
 *
 * The timeout starts when the frame is actually SENT, not while the
 * request is queued behind a predecessor, and each request chains on
 * the predecessor's SETTLEMENT — a rejected predecessor doesn't poison
 * the rest of the queue.
 */
function rpc(callId: string, msg: Record<string, unknown>): Promise<unknown> {
  const prev = rpcQueues.get(callId) ?? Promise.resolve();
  const run = prev
    .then(
      () => undefined,
      () => undefined,
    )
    .then(
      () =>
        new Promise((resolve, reject) => {
          const timeout = setTimeout(() => {
            if (pendingRpcs.has(callId)) {
              pendingRpcs.delete(callId);
              reject(
                new Error(
                  `RPC timed out after ${RPC_TIMEOUT_MS}ms (call_id=${callId})`,
                ),
              );
            }
          }, RPC_TIMEOUT_MS);
          pendingRpcs.set(callId, { resolve, reject, timeout });
          send({ ...msg, call_id: callId });
        }),
    );
  rpcQueues.set(callId, run);
  // Drop the queue entry once this tail settles (if nothing chained
  // after it) so the map doesn't hold one entry per call_id forever.
  run.then(
    () => {
      if (rpcQueues.get(callId) === run) rpcQueues.delete(callId);
    },
    () => {
      if (rpcQueues.get(callId) === run) rpcQueues.delete(callId);
    },
  );
  return run;
}

// ---------------------------------------------------------------------------
// Context builders
// ---------------------------------------------------------------------------

// Exported so the SSR runtime (ssr-runtime.ts) can build a page-facing
// `serverData` read handle that reuses this module's `send` + `pendingRpcs`
// + reader loop. The render call_id ("r_<n>") correlates DB replies back
// through the shared pendingRpcs map.
export function buildDbReader(callId: string, ssrRead = false): DbReader {
  return {
    ...buildReaderOps(callId, false, ssrRead),
    unsafe: buildReaderOps(callId, true, ssrRead),
  };
}

/**
 * Query-function caller for SSR `serverData.fn(name, args)`. Sends the same
 * `run_fn` frame `ctx.runQuery` uses, on the render's call_id — the host's
 * render loop executes the query with the PAGE's auth context (anonymous on
 * public pages) and rejects anything that isn't a query. Legacy rpc() (no
 * op_id) is correct here: concurrent calls queue settle-chained on the one
 * call_id, matching how ctx.runQuery behaves inside a function.
 */
/** `ctx.files` — signed download URLs, minted host-side. */
function buildFiles(callId: string): Files {
  return {
    async signedUrl(fileId, opts) {
      return rpc(callId, {
        type: "sign_file_url",
        file_id: fileId,
        ttl_secs: opts?.ttlSecs,
      }) as Promise<string>;
    },
  };
}

export function buildSsrFnCaller(
  callId: string,
): (name: string, args?: Record<string, unknown>) => Promise<unknown> {
  return (name, args) =>
    rpc(callId, {
      type: "run_fn",
      fn_name: name,
      fn_type: "query",
      args: args ?? {},
    });
}

function buildReaderOps(
  callId: string,
  unsafeOp: boolean,
  // `ssrRead`: true when the reader backs SSR `serverData.*` (results are
  // serialized into the client-visible `__PYLON_DATA__` blob). The Rust side
  // then applies the same per-row policy filter + `server_only`/`passwordHash`
  // projection the entity/sync read API does — UNLESS `unsafe_op` is also set
  // (`serverData.unsafe.*` stays server-trust). Server-function `ctx.db.*`
  // leaves it false and reads raw.
  ssrRead = false,
): Omit<DbReader, "unsafe"> {
  // All DB ops use rpcDb so Promise.all over ctx.db reads can run in
  // parallel without colliding on the outer call_id key.
  //
  // `unsafeOp` flag: when true, the emitted DbOp messages carry
  // `unsafe_op: true` so the Rust side knows to skip the
  // caller-aware policy gate (in Phase 2 — see
  // pylon-functions/protocol.rs). Plain ctx.db.* leaves the flag
  // off (the safe default); ctx.db.unsafe.* sets it.
  return {
    async get(entity, id) {
      return (await rpcDb(callId, {
        type: "db",
        op: "get",
        entity,
        id,
        unsafe_op: unsafeOp,
        ssr_read: ssrRead,
      })) as any;
    },
    async list(entity) {
      return (await rpcDb(callId, {
        type: "db",
        op: "list",
        entity,
        unsafe_op: unsafeOp,
        ssr_read: ssrRead,
      })) as any;
    },
    async lookup(entity, field, value) {
      return (await rpcDb(callId, {
        type: "db",
        op: "lookup",
        entity,
        field,
        value,
        unsafe_op: unsafeOp,
        ssr_read: ssrRead,
      })) as any;
    },
    async query(entity, filter) {
      return (await rpcDb(callId, {
        type: "db",
        op: "query",
        entity,
        data: filter,
        unsafe_op: unsafeOp,
        ssr_read: ssrRead,
      })) as any;
    },
    async queryGraph(query) {
      return (await rpcDb(callId, {
        type: "db",
        op: "query_graph",
        entity: "",
        data: query,
        unsafe_op: unsafeOp,
        ssr_read: ssrRead,
      })) as any;
    },
    async paginate(entity, opts) {
      // Clamp on the client side too so a caller never wastes a round trip
      // with out-of-range values. The Rust side re-clamps.
      const numItems = Math.max(1, Math.min(1000, opts.numItems | 0));
      return (await rpcDb(callId, {
        type: "db",
        op: "paginate",
        entity,
        after: opts.cursor ?? undefined,
        limit: numItems,
        unsafe_op: unsafeOp,
        ssr_read: ssrRead,
      })) as any;
    },
    async search(entity, query) {
      return (await rpcDb(callId, {
        type: "db",
        op: "search",
        entity,
        data: query,
        unsafe_op: unsafeOp,
        ssr_read: ssrRead,
      })) as any;
    },
  };
}

export function buildDbWriter(callId: string): DbWriter {
  // Top-level `ctx.db` is the safe path. `ctx.db.unsafe` is the
  // escape hatch — same surface, every emitted op carries
  // `unsafe_op: true` so the future caller-aware policy gate
  // (PYLON_STRICT_FN_POLICIES) skips enforcement. Use sparingly,
  // with a justifying comment, ideally in code that runs only
  // from server-internal callers (webhooks, cron sweeps, admin
  // tools).
  //
  // The unsafe surface carries no `.unsafe` of its own — chaining
  // is a compile error AND a self-reference would loop on JSON
  // serialization.
  return { ...buildWriterOps(callId, false), unsafe: buildWriterOps(callId, true) };
}

function buildWriterOps(callId: string, unsafeOp: boolean): Omit<DbWriter, "unsafe"> {
  return {
    ...buildReaderOps(callId, unsafeOp),
    async insert(entity, data) {
      const r = (await rpcDb(callId, {
        type: "db",
        op: "insert",
        entity,
        data,
        unsafe_op: unsafeOp,
      })) as { id: string };
      return r.id;
    },
    async update(entity, id, data) {
      const r = (await rpcDb(callId, {
        type: "db",
        op: "update",
        entity,
        id,
        data,
        unsafe_op: unsafeOp,
      })) as { updated: boolean };
      return r.updated;
    },
    async delete(entity, id) {
      const r = (await rpcDb(callId, {
        type: "db",
        op: "delete",
        entity,
        id,
        unsafe_op: unsafeOp,
      })) as { deleted: boolean };
      return r.deleted;
    },
    async link(entity, id, relation, targetId) {
      const r = (await rpcDb(callId, {
        type: "db",
        op: "link",
        entity,
        id,
        relation,
        target_id: targetId,
        unsafe_op: unsafeOp,
      })) as { linked: boolean };
      return r.linked;
    },
    async unlink(entity, id, relation) {
      const r = (await rpcDb(callId, {
        type: "db",
        op: "unlink",
        entity,
        id,
        relation,
        unsafe_op: unsafeOp,
      })) as { unlinked: boolean };
      return r.unlinked;
    },
    async advisoryLock(key) {
      // The lock key rides on `entity` to avoid carving a new field
      // for a single op. The Rust dispatcher matches on `op:
      // "advisory_lock"` and treats `entity` as the key string.
      await rpcDb(callId, {
        type: "db",
        op: "advisory_lock",
        entity: key,
        unsafe_op: unsafeOp,
      });
    },
  };
}

function buildStream(callId: string): Stream {
  return {
    write(data: string) {
      // Stream messages are fire-and-forget; they don't get a `result` reply.
      send({ type: "stream", call_id: callId, data });
    },
    writeEvent(event: string, data: string) {
      send({ type: "stream", call_id: callId, data, event });
    },
  };
}

function buildScheduler(callId: string): Scheduler {
  return {
    async runAfter(delayMs, fnName, args) {
      const r = (await rpc(callId, {
        type: "schedule",
        fn_name: fnName,
        args,
        delay_ms: delayMs,
      })) as { id?: string };
      return r.id || "";
    },
    async runAt(timestamp, fnName, args) {
      const r = (await rpc(callId, {
        type: "schedule",
        fn_name: fnName,
        args,
        run_at: timestamp,
      })) as { id?: string };
      return r.id || "";
    },
    async cancel(scheduleId) {
      await rpc(callId, {
        type: "cancel_schedule",
        schedule_id: scheduleId,
      });
    },
  };
}

/**
 * Build the email sender that round-trips through the host runtime.
 *
 * Each `send` emits a `send_email` protocol message; the runtime
 * forwards to whatever transport PYLON_EMAIL_PROVIDER points at and
 * replies success or error. Errors arrive as thrown exceptions on
 * the action's await, just like every other RPC. No silent failures.
 */
// Mirrors the Rust-side guard in the send_email dispatch arm: attachments
// ride one NDJSON line, so an oversized payload is an unbounded allocation
// on both sides of the pipe. Reject before any framing happens.
const EMAIL_MAX_ATTACHMENT_B64_BYTES = 15 * 1024 * 1024;
const EMAIL_MAX_ATTACHMENTS = 20;

function buildEmail(callId: string): EmailSender {
  async function send(
    toOrOptions: string | EmailOptions,
    subject?: string,
    body?: string,
  ): Promise<void> {
    // Options form: first argument is the message object.
    if (typeof toOrOptions === "object" && toOrOptions !== null) {
      const opts = toOrOptions;
      const attachments = opts.attachments ?? [];
      if (attachments.length > EMAIL_MAX_ATTACHMENTS) {
        throw new Error(
          `ctx.email.send: ${attachments.length} attachments exceeds the limit of ${EMAIL_MAX_ATTACHMENTS} per email`,
        );
      }
      const b64Total = attachments.reduce(
        (n: number, a: EmailAttachment) => n + a.content.length,
        0,
      );
      if (b64Total > EMAIL_MAX_ATTACHMENT_B64_BYTES) {
        throw new Error(
          `ctx.email.send: attachments total ${b64Total} base64 bytes, exceeding the ${EMAIL_MAX_ATTACHMENT_B64_BYTES}-byte limit (≈11MB of raw file data)`,
        );
      }
      await rpc(callId, {
        type: "send_email",
        to: opts.to,
        subject: opts.subject,
        body: opts.text,
        html: opts.html,
        // Wire fields are snake_case; contentType is the TS-facing name.
        attachments: attachments.map((a: EmailAttachment) => ({
          filename: a.filename,
          content_type: a.contentType,
          content: a.content,
        })),
      });
      return;
    }
    // Positional form: send(to, subject, body).
    await rpc(callId, {
      type: "send_email",
      to: toOrOptions,
      subject,
      body,
    });
  }
  return { send } as EmailSender;
}

/**
 * Build the LLM client that round-trips through the host runtime.
 *
 * Each call emits an `llm_complete` protocol message; the runtime
 * forwards to the configured provider (PYLON_LLM_PROVIDER) and replies
 * with the parsed response. The host enforces model-allowlist gating
 * for non-admin callers — that's why the API key never leaves the
 * server process.
 *
 * Errors carry an `err.code` so handlers can branch on `LLM_NOT_CONFIGURED`
 * vs `PROVIDER_HTTP_429` vs `MODEL_NOT_ALLOWED` without parsing message
 * strings.
 */
export function buildLlm(callId: string): Llm {
  return {
    async complete(request: LlmCompleteRequest): Promise<LlmCompleteResponse> {
      return (await rpc(callId, {
        type: "llm_complete",
        request,
      })) as LlmCompleteResponse;
    },

    async stream(
      request: LlmCompleteRequest,
      onEvent: (event: LlmStreamEvent) => void,
    ): Promise<LlmCompleteResponse> {
      return (await rpcStreaming(
        callId,
        { type: "llm_stream", request },
        (event) => onEvent(event as LlmStreamEvent),
      )) as LlmCompleteResponse;
    },
  };
}

/**
 * Build the room broadcaster. One `room_broadcast` message per call;
 * the host fans the event out through the same RoomManager the
 * `/api/rooms/*` routes use, so a server push and a member push land
 * on subscribers identically.
 */
export function buildRooms(callId: string): Rooms {
  return {
    async broadcast(room: string, topic: string, data?: unknown) {
      return (await rpcDb(callId, {
        type: "room_broadcast",
        room,
        topic,
        data: data ?? {},
      })) as { delivered: boolean };
    },
  };
}

/**
 * Build the connection registry that round-trips through the host.
 * Each method emits a `{type:"connection", op:"..."}` message;
 * the host's ConnectionManager runs the actual OAuth flow + DB
 * read/write and returns the typed reply.
 */
function buildConnections(callId: string): Connections {
  return {
    async authorizeUrl(name, opts) {
      const data = (await rpc(callId, {
        type: "connection",
        op: "authorize_url",
        payload: {
          name,
          post_redirect: opts?.postRedirect,
        },
      })) as { url: string };
      return data;
    },
    async get(name) {
      const data = (await rpc(callId, {
        type: "connection",
        op: "get",
        payload: { name },
      })) as { access_token: string; scope: string | null; expires_at: number | null };
      return {
        accessToken: data.access_token,
        scope: data.scope,
        expiresAt: data.expires_at,
      };
    },
    async list() {
      const data = (await rpc(callId, {
        type: "connection",
        op: "list",
        payload: {},
      })) as {
        connections: Array<{
          name: string;
          provider: string;
          scope: string | null;
          expires_at: number | null;
          updated_at: number;
        }>;
      };
      return {
        connections: data.connections.map((c) => ({
          name: c.name,
          provider: c.provider,
          scope: c.scope,
          expiresAt: c.expires_at,
          updatedAt: c.updated_at,
        })),
      };
    },
    async disconnect(name) {
      return (await rpc(callId, {
        type: "connection",
        op: "disconnect",
        payload: { name },
      })) as { disconnected: boolean };
    },
  };
}

function buildActionCtx(
  callId: string,
  auth: AuthInfo,
  stream: Stream,
  scheduler: Scheduler,
  email: EmailSender,
  llm: Llm,
  rooms: Rooms,
  connections: Connections,
  request?: unknown
): ActionCtx {
  // The host sends `request` as snake_case JSON (`raw_body`); normalize it
  // to the camelCase shape documented in ActionCtx so action authors don't
  // have to care about the transport. Absent when invoked programmatically.
  let normalizedRequest: ActionCtx["request"];
  if (request && typeof request === "object") {
    const r = request as Record<string, unknown>;
    normalizedRequest = {
      method: String(r.method ?? ""),
      path: String(r.path ?? ""),
      headers: (r.headers as Record<string, string>) ?? {},
      rawBody: String(r.raw_body ?? r.rawBody ?? ""),
    };
  }
  return {
    auth,
    stream,
    scheduler,
    email,
    llm,
    rooms,
    connections,
    env: process.env as Record<string, string>,
    async runQuery(fnName, args) {
      return rpc(callId, {
        type: "run_fn",
        fn_name: fnName,
        fn_type: "query",
        args,
      }) as Promise<any>;
    },
    async runMutation(fnName, args) {
      return rpc(callId, {
        type: "run_fn",
        fn_name: fnName,
        fn_type: "mutation",
        args,
      }) as Promise<any>;
    },
    error(code, message) {
      const err = new Error(message);
      (err as any).code = code;
      return err;
    },
    files: buildFiles(callId),
    // Actions have no ctx.db; read membership via the built-in internal query.
    requireMember: makeRequireMember(auth.userId, (entity, filter) =>
      rpc(callId, {
        type: "run_fn",
        fn_name: "__pylonMemberLookup",
        fn_type: "query",
        args: { entity, filter },
      }) as Promise<any[]>,
    ),
    request: normalizedRequest,
  };
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

const registry = new Map<string, FnDefinition>();

// Built-in internal query backing `ctx.requireMember` on ACTION ctx — actions
// have no `ctx.db`, so they read membership via `runQuery` to this. Registered
// before app fns load (so the host sees it in the registration list and an
// action's runQuery dispatches back here); app fns load by filename basename so
// they can't collide with this reserved name. The read is policy-gated like any
// ctx.db read, which is fine: a member can always read their own membership row.
registry.set("__pylonMemberLookup", {
  type: "query",
  internal: true,
  handler: async (ctx: any, args: any) =>
    ctx.db.query(args.entity, { ...(args.filter ?? {}), $limit: 1 }),
} as FnDefinition);

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

async function handleCall(msg: CallMessage): Promise<void> {
  const def = registry.get(msg.fn_name);

  if (!def) {
    send({
      type: "error",
      call_id: msg.call_id,
      code: "FN_NOT_FOUND",
      message: `Function "${msg.fn_name}" not registered`,
    });
    return;
  }

  // Enforce that the caller's declared fn_type matches what's registered.
  // Without this, a buggy or malicious peer could label a mutation call as
  // a query and break host-side assumptions about side effects / auth.
  // Accept msg.fn_type undefined for backwards compatibility — the host
  // always sends it in current versions.
  if (msg.fn_type && msg.fn_type !== def.type) {
    send({
      type: "error",
      call_id: msg.call_id,
      code: "FN_TYPE_MISMATCH",
      message: `Function "${msg.fn_name}" is registered as ${def.type}, not ${msg.fn_type}`,
    });
    return;
  }

  if (def.args) {
    const { valid, errors } = validateArgs(msg.args, def.args);
    if (!valid) {
      send({
        type: "error",
        call_id: msg.call_id,
        code: "INVALID_ARGS",
        message: errors.join("; "),
      });
      return;
    }
  }

  const stream = buildStream(msg.call_id);
  const scheduler = buildScheduler(msg.call_id);
  const email = buildEmail(msg.call_id);
  const llm = buildLlm(msg.call_id);
  const rooms = buildRooms(msg.call_id);
  const connections = buildConnections(msg.call_id);

  // Normalize the Rust-side auth envelope (snake_case) to the camelCase
  // shape that AuthInfo documents. Handlers read `ctx.auth.userId`; the
  // wire uses `user_id`. Without this adapter every handler's
  // `if (!ctx.auth.userId)` check fires and authenticated calls come
  // back as UNAUTHENTICATED. Accept both shapes so old TS runtimes that
  // already got camelCase don't regress.
  const rawAuth = msg.auth as unknown as Record<string, unknown>;
  const auth: AuthInfo = {
    ...normalizeAuthClaims(rawAuth),
    // `elevate` round-trips through the host runtime which mutates
    // the per-call caller_is_admin flag — that's what subsequent
    // scheduler.runAfter() reads. We also mutate the local
    // `auth.isAdmin` so handler code that re-checks `ctx.auth.isAdmin`
    // after elevation sees the new value (it would otherwise stay
    // false even though scheduling now works, which is confusing).
    async elevate(options: { admin: boolean; reason: string }) {
      await rpc(msg.call_id, {
        type: "elevate_auth",
        admin: options.admin,
        reason: options.reason,
      });
      if (options.admin) {
        // Mutate in-place. AuthInfo isn't frozen and handlers hold a
        // reference, so the read on the next line of their code
        // reflects the elevated state.
        (auth as { isAdmin: boolean }).isAdmin = true;
      }
    },
  };

  // Env is read-only config — safe to expose on every ctx variant. Without
  // this, queries/mutations that need a config flag have to be declared as
  // actions just to reach `process.env`, which is a footgun: the failure
  // mode is `ctx.env.X` throwing "cannot read properties of undefined" at
  // runtime, with no compile-time hint.
  const env = process.env as Record<string, string>;

  let ctx: QueryCtx | MutationCtx | ActionCtx;
  switch (def.type) {
    case "query": {
      // ctx.llm is intentionally absent on queries — reactive
      // re-runs would re-bill the LLM call on every dep change.
      // Move LLM calls into actions / mutations.
      const reader = buildDbReader(msg.call_id);
      ctx = {
        db: reader,
        auth,
        env,
        // No transaction to roll back here — this is purely so a query
        // can fail with a code the caller can branch on.
        error(code, message) {
          const err = new Error(message);
          (err as any).code = code;
          return err;
        },
        requireMember: makeRequireMember(auth.userId, (entity, filter) =>
          reader.query(entity, { ...filter, $limit: 1 }),
        ),
        files: buildFiles(msg.call_id),
      };
      break;
    }
    case "mutation": {
      const writer = buildDbWriter(msg.call_id);
      ctx = {
        db: writer,
        auth,
        stream,
        scheduler,
        env,
        llm,
        rooms,
        connections,
        error(code, message) {
          const err = new Error(message);
          (err as any).code = code;
          return err;
        },
        requireMember: makeRequireMember(auth.userId, (entity, filter) =>
          writer.query(entity, { ...filter, $limit: 1 }),
        ),
        files: buildFiles(msg.call_id),
      };
      break;
    }
    case "action":
      ctx = buildActionCtx(
        msg.call_id,
        auth,
        stream,
        scheduler,
        email,
        llm,
        rooms,
        connections,
        (msg as unknown as { request?: unknown }).request,
      );
      break;
  }

  try {
    const result = await def.handler(ctx, msg.args);
    send({
      type: "return",
      call_id: msg.call_id,
      value: result ?? null,
    });
  } catch (err: any) {
    // Redact. Handler errors historically shipped raw `err.message` to the
    // caller, which leaked DB error text, stack-trace-looking strings, and
    // internal concurrency-invariant messages. Authors can still surface a
    // caller-safe message by throwing with an explicit `code` AND a message
    // they're willing to disclose: `ctx.error(code, message)` uses that
    // pattern. Anything else gets a generic message; the full error is
    // logged to stderr where the operator can see it.
    const hasExplicitCode = typeof err?.code === "string" && err.code.length > 0;
    if (hasExplicitCode) {
      send({
        type: "error",
        call_id: msg.call_id,
        code: err.code,
        message:
          typeof err.message === "string" && err.message.length > 0
            ? err.message
            : "Handler error",
      });
    } else {
      // No explicit code — assume it's an unexpected Error/thrown value.
      // Log the real error to stderr (server operator visible) and return
      // a safe placeholder to the client. In DEV the real message (and
      // top stack frame) rides along: the developer debugging a 500 IS
      // the operator, and hiding the reason from the HTTP response just
      // sends them (or their agent) digging through server logs for
      // something we already know. Production responses stay masked.
      console.error(
        `[functions] unhandled error in ${msg.fn_name} (${msg.call_id}):`,
        err,
      );
      const devDetail =
        isDevMode() && typeof err?.message === "string" && err.message.length > 0
          ? ` (dev): ${err.message}${firstStackFrame(err)}`
          : "";
      send({
        type: "error",
        call_id: msg.call_id,
        code: "HANDLER_ERROR",
        message: `Internal handler error${devDetail}`,
      });
    }
  }
}

/**
 * The first user-code stack frame of an error, for dev-mode error
 * detail — one frame locates the throw without shipping a whole trace.
 */
function firstStackFrame(err: unknown): string {
  const stack = (err as { stack?: string })?.stack;
  if (typeof stack !== "string") return "";
  const frame = stack
    .split("\n")
    .map((l) => l.trim())
    .find((l) => l.startsWith("at "));
  return frame ? ` [${frame}]` : "";
}

// ---------------------------------------------------------------------------
// Startup: scan functions dir, send ready, then start reader loop
// ---------------------------------------------------------------------------

async function main() {
  // Fence user `console.*` away from stdout BEFORE any user code is
  // imported — the import side-effects alone could print a stray line
  // that the host parses as a protocol frame.
  fenceStdout();

  const fnDir = process.argv[2] || "./functions";

  let files: string[];
  try {
    files = readdirSync(fnDir).filter(
      (f) => f.endsWith(".ts") || f.endsWith(".js")
    );
  } catch {
    // No `functions/` directory. Legitimate for a pure-SSR app (file-based
    // `app/**/page.tsx` routes + entity CRUD, no server functions) — the host
    // still spawns this runner to execute SSR renders. Load zero functions and
    // fall through so we send `ready` AND start the reader loop; returning here
    // would leave the runner unable to serve renders (silent 404s).
    files = [];
  }

  for (const file of files) {
    const name = basename(file, file.endsWith(".ts") ? ".ts" : ".js");
    try {
      const mod = await import(join(process.cwd(), fnDir, file));
      const def = mod.default as FnDefinition | undefined;
      // Runtime shape check — a misnamed/malformed export should
      // log + skip, not crash the loader. TS narrows `def.handler`
      // as always-defined because the FnDefinition type says so,
      // but at runtime we don't know what the user actually exported.
      const anyDef = def as unknown as Record<string, unknown> | undefined;
      if (
        anyDef &&
        typeof anyDef.type === "string" &&
        typeof anyDef.handler === "function"
      ) {
        registry.set(name, def as FnDefinition);
      }
    } catch (err) {
      console.error(`[functions] Failed to load ${file}:`, err);
    }
  }

  const functions = Array.from(registry.entries()).map(([name, def]) => ({
    name,
    fn_type: def.type,
    args_schema: def.args || null,
    // Whether the function is callable only via runQuery/runMutation/
    // runAction from another function. The Rust router refuses /api/fn
    // requests for internal fns; the Bun runtime here doesn't gate
    // (nested calls go through the same dispatcher).
    internal: def.internal === true,
    // Declarative auth gate, enforced by the Rust router before the
    // handler is invoked. Defaults to "user" when the TS def omits
    // it — secure by default. See `packages/functions/src/define.ts`
    // for the developer-facing AuthMode docs.
    auth: def.auth ?? "user",
    // Per-function call deadline in seconds. Null → the host uses its
    // global PYLON_FN_CALL_TIMEOUT default. Drives both the call deadline
    // and the wedge backstop for this function's worker.
    timeout_secs:
      typeof def.timeout === "number" && def.timeout > 0
        ? Math.floor(def.timeout)
        : null,
  }));
  send({ type: "ready", functions });

  // Belt-and-suspenders against orphaning: if the host dies in a way that
  // somehow leaves our stdin open, we'll have been REPARENTED — our ppid
  // changes (to init or the nearest subreaper). Compare against the ppid we
  // were born with rather than testing `ppid === 1`: in a container the
  // host pylon usually IS PID 1, so every healthy runner is born with
  // ppid 1 and the equality check kills the whole pool in a 2s respawn
  // loop. Unref'd so the watch never keeps us alive on its own.
  const initialPpid = process.ppid;
  const orphanWatch = setInterval(() => {
    if (process.ppid !== initialPpid) process.exit(0);
  }, 2000);
  if (typeof orphanWatch.unref === "function") orphanWatch.unref();

  await readerLoop();

  // readerLoop only returns when stdin hits EOF — i.e. the host (the pylon
  // process that spawned us) is gone. Force-exit. We must NOT rely on the
  // event loop draining on its own: the stdout writer, keep-alive sockets
  // from `fetch`, and Bun's own handles keep the process alive, so every
  // killed `pylon dev` would otherwise orphan its whole bun runner pool.
  process.exit(0);
}

main().catch((err) => {
  console.error("[functions] Fatal error:", err);
  process.exit(1);
});
