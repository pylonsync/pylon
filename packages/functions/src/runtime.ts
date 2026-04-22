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
  Stream,
  Scheduler,
  QueryCtx,
  MutationCtx,
  ActionCtx,
  FnDefinition,
  AuthInfo,
} from "./types";
import { validateArgs } from "./validators";
import { readdirSync } from "fs";
import { join, basename } from "path";

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

function send(msg: Record<string, unknown>): void {
  const line = JSON.stringify(msg) + "\n";
  Bun.write(Bun.stdout, line);
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

let opSeq = 0;
function nextOpId(callId: string): string {
  opSeq += 1;
  return `${callId}#${opSeq}`;
}

/**
 * Upper bound on how long an individual host → TS RPC (e.g. `ctx.db.get`)
 * can wait for a reply. The Rust side enforces its own per-handler timeout
 * (STATECRAFT_FN_CALL_TIMEOUT, default 30s), but if a protocol frame gets
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
 * RPC for non-db protocol replies (scheduler.runAfter, nested function
 * calls, etc.) where at-most-one in-flight per call_id is the right
 * contract. Keeps the legacy keying so these reply shapes don't need
 * op_id support on the host.
 */
function rpc(callId: string, msg: Record<string, unknown>): Promise<unknown> {
  return new Promise((resolve, reject) => {
    if (pendingRpcs.has(callId)) {
      reject(
        new Error(
          `Internal: concurrent RPC attempted on same call_id (${callId})`,
        ),
      );
      return;
    }
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
  });
}

// ---------------------------------------------------------------------------
// Context builders
// ---------------------------------------------------------------------------

function buildDbReader(callId: string): DbReader {
  // All DB ops use rpcDb so Promise.all over ctx.db reads can run in
  // parallel without colliding on the outer call_id key.
  return {
    async get(entity, id) {
      return (await rpcDb(callId, { type: "db", op: "get", entity, id })) as any;
    },
    async list(entity) {
      return (await rpcDb(callId, { type: "db", op: "list", entity })) as any;
    },
    async lookup(entity, field, value) {
      return (await rpcDb(callId, {
        type: "db",
        op: "lookup",
        entity,
        field,
        value,
      })) as any;
    },
    async query(entity, filter) {
      return (await rpcDb(callId, {
        type: "db",
        op: "query",
        entity,
        data: filter,
      })) as any;
    },
    async queryGraph(query) {
      return (await rpcDb(callId, {
        type: "db",
        op: "query_graph",
        entity: "",
        data: query,
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
      })) as any;
    },
  };
}

function buildDbWriter(callId: string): DbWriter {
  const reader = buildDbReader(callId);
  return {
    ...reader,
    async insert(entity, data) {
      const r = (await rpcDb(callId, {
        type: "db",
        op: "insert",
        entity,
        data,
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
      })) as { updated: boolean };
      return r.updated;
    },
    async delete(entity, id) {
      const r = (await rpcDb(callId, {
        type: "db",
        op: "delete",
        entity,
        id,
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
      })) as { unlinked: boolean };
      return r.unlinked;
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

function buildActionCtx(
  callId: string,
  auth: AuthInfo,
  stream: Stream,
  scheduler: Scheduler,
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
    request: normalizedRequest,
  };
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

const registry = new Map<string, FnDefinition>();

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

  // Normalize the Rust-side auth envelope (snake_case) to the camelCase
  // shape that AuthInfo documents. Handlers read `ctx.auth.userId`; the
  // wire uses `user_id`. Without this adapter every handler's
  // `if (!ctx.auth.userId)` check fires and authenticated calls come
  // back as UNAUTHENTICATED. Accept both shapes so old TS runtimes that
  // already got camelCase don't regress.
  const rawAuth = msg.auth as unknown as Record<string, unknown>;
  const auth: AuthInfo = {
    userId: ((rawAuth.userId ?? rawAuth.user_id) as string | null | undefined) ?? null,
    isAdmin: Boolean(rawAuth.isAdmin ?? rawAuth.is_admin),
  };

  let ctx: QueryCtx | MutationCtx | ActionCtx;
  switch (def.type) {
    case "query":
      ctx = { db: buildDbReader(msg.call_id), auth };
      break;
    case "mutation":
      ctx = {
        db: buildDbWriter(msg.call_id),
        auth,
        stream,
        scheduler,
        error(code, message) {
          const err = new Error(message);
          (err as any).code = code;
          return err;
        },
      };
      break;
    case "action":
      // Pass `msg.request` so actions invoked via `defineRoute` HTTP
      // bindings can reach raw headers + body (for webhook signature
      // verification). Programmatic invocations (runAction, jobs) get
      // undefined here and `ctx.request` reads as undefined — the type
      // is optional on purpose.
      ctx = buildActionCtx(
        msg.call_id,
        auth,
        stream,
        scheduler,
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
      // a safe placeholder to the client.
      console.error(
        `[functions] unhandled error in ${msg.fn_name} (${msg.call_id}):`,
        err,
      );
      send({
        type: "error",
        call_id: msg.call_id,
        code: "HANDLER_ERROR",
        message: "Internal handler error",
      });
    }
  }
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
    send({
      type: "ready",
      functions: [],
      error: `Cannot read functions directory: ${fnDir}`,
    });
    return;
  }

  for (const file of files) {
    const name = basename(file, file.endsWith(".ts") ? ".ts" : ".js");
    try {
      const mod = await import(join(process.cwd(), fnDir, file));
      const def = mod.default as FnDefinition;
      if (def && def.type && def.handler) {
        registry.set(name, def);
      }
    } catch (err) {
      console.error(`[functions] Failed to load ${file}:`, err);
    }
  }

  const functions = Array.from(registry.entries()).map(([name, def]) => ({
    name,
    fn_type: def.type,
    args_schema: def.args || null,
  }));
  send({ type: "ready", functions });

  await readerLoop();
}

main().catch((err) => {
  console.error("[functions] Fatal error:", err);
  process.exit(1);
});
