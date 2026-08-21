/**
 * `agent()` — a define-type for LLM agents with durable run state.
 *
 * ```ts
 * // functions/researcher.ts
 * export default agent({
 *   system: "You are a research assistant.",
 *   tools: {
 *     searchDocs: {
 *       description: "Search the document library",
 *       args: { query: v.string() },
 *       handler: async (ctx, { query }) => ctx.runQuery("findSimilar", { query }),
 *     },
 *   },
 * });
 * ```
 *
 * An agent compiles to an ordinary streaming ACTION (named after its
 * file, callable via `streamFn("researcher", { input })`), whose
 * handler runs the tool loop:
 *
 *   1. Create an `AgentRun` row (or load one when `runId` is passed —
 *      that's how a conversation continues) and append the user's
 *      input as an `AgentMessage`.
 *   2. `ctx.llm.stream` with the declared tools; text deltas flow to
 *      `ctx.stream` (resumable — the run row records the stream id).
 *   3. On `stop_reason === "tool_use"`: validate each tool call's
 *      input against its validators, run the handler, record the
 *      call + result as messages, loop.
 *   4. Terminal: run marked completed/failed.
 *
 * `AgentRun` and `AgentMessage` are real synced entities (injected
 * into the manifest by the SDK when any agent exists), owner-scoped by
 * policy — so `db.useQuery("AgentMessage", { where: { runId } })`
 * shows the transcript live on every one of the user's devices,
 * including tool calls, with zero extra plumbing.
 */

import { action } from "./define";
import { v, validateArgs } from "./validators";
import type {
  ActionCtx,
  AnyValidator,
  FnDefinition,
  LlmContentBlock,
  LlmMessage,
  LlmTool,
  ValidatorSchema,
} from "./types";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** One tool an agent can call. */
export interface AgentTool {
  /** Shown to the model — say when to use the tool, not how it works. */
  description: string;
  /** Argument validators (same `v.*` schema as functions). The model's
   *  JSON is validated before the handler runs; invalid input becomes
   *  a tool_result error the model can react to. Omit for no-arg tools. */
  args?: ValidatorSchema;
  /** Runs with the agent action's ctx (runQuery/runMutation, llm,
   *  email, …). The return value is JSON-serialized into the
   *  tool_result the model sees. Throwing marks the result is_error —
   *  the model sees the message and can recover. */
  handler: (ctx: ActionCtx, input: Record<string, unknown>) => unknown;
}

export interface AgentDefinition {
  /** System prompt. A function receives the ctx + call args for
   *  per-user prompts. */
  system?: string | ((ctx: ActionCtx, args: AgentCallArgs) => string);
  tools?: Record<string, AgentTool>;
  /** Model override (subject to the server's allowlist). */
  model?: string;
  /** Max model↔tool round-trips per invocation (default 64). Hitting
   *  the cap fails the run rather than looping forever. Steering input
   *  drained mid-invocation extends the same invocation, so this bounds
   *  a steered turn too. The run row's cumulative `steps` counts every
   *  invocation and is not capped. */
  maxSteps?: number;
  /** max_tokens per completion (default: server default). */
  maxTokens?: number;
  /** Auth gate for the action (default "user" — runs are owner-scoped,
   *  so an authenticated caller is the natural default). */
  auth?: "user" | "admin";
  /** Idle timeout in seconds (default 600; activity extends it). */
  timeout?: number;
}

/** The synthesized action's args. */
export interface AgentCallArgs {
  /** The user's message for this turn. Required unless `cancel`. */
  input?: string;
  /** Continue an existing run (must belong to the caller and this
   *  agent). Omit to start a new run. Sending while the run is already
   *  generating queues the message for that generation rather than
   *  refusing it — see {@link AgentResult.queued}. */
  runId?: string;
  /** Optional display title, stored on new runs. */
  title?: string;
  /** Ask the run to stop. Requires `runId`, ignores `input`, and
   *  returns as soon as the request is recorded — a live generation
   *  stops at its next turn boundary. */
  cancel?: boolean;
}

/** What the agent action resolves with (also the `event: result`
 *  payload on the SSE stream). */
export interface AgentResult {
  runId: string;
  /** Concatenated text of the final assistant message. */
  text: string;
  /** Round-trips consumed by THIS invocation. */
  steps: number;
  usage: { input_tokens: number; output_tokens: number };
  /** The message was queued onto a generation already in flight
   *  instead of starting a turn. No model call happened on this call;
   *  the running loop picks the message up at its next boundary. */
  queued?: boolean;
  /** The run stopped because cancel was requested. `text` holds
   *  whatever the model had produced by then. */
  cancelled?: boolean;
}

// ---------------------------------------------------------------------------
// Validators → JSON Schema (for LlmTool.input_schema)
// ---------------------------------------------------------------------------

/** Convert one `v.*` validator to a JSON-Schema fragment. */
export function validatorToJsonSchema(val: AnyValidator): Record<string, unknown> {
  const t = (val as { type: string }).type;
  switch (t) {
    case "string":
      return { type: "string" };
    case "int":
      return { type: "integer" };
    case "number":
      return { type: "number" };
    case "boolean":
      return { type: "boolean" };
    case "null":
      return { type: "null" };
    case "id":
      return { type: "string" };
    case "literal":
      return { const: (val as { value: unknown }).value };
    case "array":
      return {
        type: "array",
        items: validatorToJsonSchema((val as { items: AnyValidator }).items),
      };
    case "object": {
      const fields = (val as { fields: ValidatorSchema }).fields ?? {};
      return validatorSchemaToJsonSchema(fields);
    }
    case "union": {
      const variants = (val as { variants: AnyValidator[] }).variants ?? [];
      return { anyOf: variants.map(validatorToJsonSchema) };
    }
    // json / any — anything goes; the empty schema is JSON Schema's
    // "any value".
    default:
      return {};
  }
}

/** Convert a validator schema (a tool's `args`) to a JSON-Schema
 *  object with `required` derived from non-optional fields. */
export function validatorSchemaToJsonSchema(
  schema: ValidatorSchema,
): Record<string, unknown> {
  const properties: Record<string, unknown> = {};
  const required: string[] = [];
  for (const [name, val] of Object.entries(schema)) {
    properties[name] = validatorToJsonSchema(val as AnyValidator);
    if (!(val as { optional?: boolean }).optional) {
      required.push(name);
    }
  }
  const out: Record<string, unknown> = { type: "object", properties };
  if (required.length > 0) out.required = required;
  return out;
}

// ---------------------------------------------------------------------------
// The define-type
// ---------------------------------------------------------------------------

/** Marker so the SDK's discoverFunctions can detect agents and inject
 *  the AgentRun/AgentMessage entities into the manifest. */
export const AGENT_MARKER = "__pylonAgent";

export function agent(def: AgentDefinition): FnDefinition<AgentCallArgs, AgentResult> {
  const fnDef = action({
    args: {
      input: v.optional(v.string()),
      runId: v.optional(v.string()),
      title: v.optional(v.string()),
      cancel: v.optional(v.boolean()),
    },
    auth: def.auth ?? "user",
    timeout: def.timeout ?? 600,
    handler: (ctx: ActionCtx, args: AgentCallArgs) =>
      runAgentLoop(def, ctx, args),
  } as never) as FnDefinition<AgentCallArgs, AgentResult>;
  (fnDef as unknown as Record<string, unknown>)[AGENT_MARKER] = true;
  return fnDef;
}

export function isAgentDefinition(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as Record<string, unknown>)[AGENT_MARKER] === true
  );
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

const DEFAULT_MAX_STEPS = 64;

/** How often the loop asks whether cancel was requested while a tool
 *  handler is running. Only runs during a tool batch: the host blocks
 *  its per-call read loop for the whole of `ctx.llm.stream`, so no RPC
 *  the child issues during a generation would be serviced anyway. */
const CANCEL_POLL_MS = 2000;

/** Tool results persist into AgentMessage rows and replay into every
 *  later completion's context — cap them so one oversized return can't
 *  bloat the transcript (and the sync payloads) unboundedly. */
const MAX_TOOL_RESULT_CHARS = 64 * 1024;

function truncateToolResult(s: string): string {
  if (s.length <= MAX_TOOL_RESULT_CHARS) return s;
  return `${s.slice(0, MAX_TOOL_RESULT_CHARS)}\n[truncated]`;
}

interface StoredMessage {
  id: string;
  seq: number;
  role: string;
  content: unknown;
}

/** What one turn-boundary `drainInput` reports back. */
interface DrainResult {
  input: string[];
  cancelRequested: boolean;
  completed: boolean;
}

/** Give tool handlers a ctx whose `signal` also aborts on cancel, so a
 *  handler that already threads `ctx.signal` into fetch stops with the
 *  run. Prototype-based rather than a spread: ctx's methods keep
 *  working when invoked on the derived object. */
function ctxWithSignal(ctx: ActionCtx, signal: AbortSignal): ActionCtx {
  const combined =
    ctx.signal && typeof AbortSignal.any === "function"
      ? AbortSignal.any([ctx.signal, signal])
      : signal;
  const derived = Object.create(ctx) as ActionCtx;
  Object.defineProperty(derived, "signal", {
    value: combined,
    enumerable: true,
  });
  return derived;
}

async function runAgentLoop(
  def: AgentDefinition,
  ctx: ActionCtx,
  args: AgentCallArgs,
): Promise<AgentResult> {
  // The action's file-inferred name isn't visible here; the internal
  // write mutation derives it from this marker arg set by the registry
  // loader (see registerAgentInternals). Fallback "agent".
  const agentName =
    (args as unknown as Record<string, unknown>).__agentName?.toString() ??
    "agent";

  const noUsage = { input_tokens: 0, output_tokens: 0 };

  // Cancel is a control call, not a turn: record the request and
  // return. Ownership (and the agent match) is enforced inside the
  // internal mutation, which runs under this caller's auth.
  if (args.cancel === true) {
    if (!args.runId) {
      throw ctx.error(
        "AGENT_CANCEL_NEEDS_RUN",
        "cancel requires the runId of the run to stop",
      );
    }
    await ctx.runMutation("__pylon_agent_write", {
      op: "requestCancel",
      runId: args.runId,
      agent: agentName,
    });
    return {
      runId: args.runId,
      text: "",
      steps: 0,
      usage: noUsage,
      cancelled: true,
    };
  }

  const input = args.input ?? "";
  if (input === "") {
    throw ctx.error(
      "AGENT_INPUT_REQUIRED",
      "input is required (pass cancel: true to stop a run instead)",
    );
  }

  // A run left "running" longer than the agent's timeout is a dead
  // generation (the process died before the terminal status write) —
  // continuations may take it over instead of waiting forever.
  const staleMs = Math.max(def.timeout ?? 600, 60) * 1000;

  // 1. Create or load the run (ownership enforced inside the internal
  // fns, which run under this caller's auth).
  let runId: string;
  let history: LlmMessage[] = [];
  let priorSteps = 0;
  if (args.runId) {
    const loaded = await ctx.runQuery<{
      run: {
        id: string;
        agent: string;
        status: string;
        updatedAt?: string;
        steps?: number;
      };
      messages: StoredMessage[];
    }>("__pylon_agent_read", { runId: args.runId });
    if (loaded.run.agent !== agentName) {
      throw ctx.error(
        "AGENT_MISMATCH",
        `Run ${args.runId} belongs to agent "${loaded.run.agent}"`,
      );
    }
    if (loaded.run.status === "running") {
      const updatedAt = Date.parse(String(loaded.run.updatedAt ?? "")) || 0;
      if (Date.now() - updatedAt < staleMs) {
        // The generation is alive. Hand it the message rather than
        // refusing the call: the loop drains the queue at its next
        // turn boundary, which is what lets a user steer mid-run.
        const queued = await ctx.runMutation<{ queued: number }>(
          "__pylon_agent_write",
          { op: "enqueueInput", runId: loaded.run.id, input },
        );
        ctx.stream.writeEvent(
          "queued",
          JSON.stringify({ runId: loaded.run.id, queued: queued.queued }),
        );
        return {
          runId: loaded.run.id,
          text: "",
          steps: 0,
          usage: noUsage,
          queued: true,
        };
      }
    }
    runId = loaded.run.id;
    priorSteps = Number(loaded.run.steps) || 0;
    history = loaded.messages.map(storedToLlmMessage);
  } else {
    const created = await ctx.runMutation<{ id: string }>(
      "__pylon_agent_write",
      { op: "createRun", agent: agentName, title: args.title ?? null },
    );
    runId = created.id;
  }

  const write = (op: Record<string, unknown>) =>
    ctx.runMutation<Record<string, unknown>>("__pylon_agent_write", {
      ...op,
      runId,
    });

  // 2. Claim the run FIRST (guarded inside the mutation's transaction —
  // the pre-flight check above races between concurrent turns), then
  // write. The stream id lets other devices attach mid-generation.
  await write({
    op: "setStatus",
    status: "running",
    streamId: ctx.stream.id ?? null,
    guardNotRunning: true,
    staleMs,
  });

  // A crash between persisting an assistant tool_use turn and its
  // tool_results leaves a transcript the LLM API rejects on replay.
  // Repair with synthetic error results, carried in the SAME user
  // message as this turn's input: two user messages in a row are
  // rejected as well, so appending the repair separately would trade
  // one unreplayable transcript for another.
  let repairs: LlmContentBlock[] = [];
  const lastMsg = history[history.length - 1];
  if (lastMsg?.role === "assistant" && Array.isArray(lastMsg.content)) {
    repairs = lastMsg.content
      .filter(
        (b): b is Extract<LlmContentBlock, { type: "tool_use" }> =>
          (b as { type?: string }).type === "tool_use",
      )
      .map((b) => ({
        type: "tool_result",
        tool_use_id: b.id,
        content: "tool execution was interrupted",
        is_error: true,
      }));
  }
  // Plain string when there is nothing to repair — the common case
  // stays a simple transcript row.
  const opening: string | LlmContentBlock[] =
    repairs.length > 0
      ? [...repairs, { type: "text", text: input }]
      : input;
  await write({ op: "appendMessage", role: "user", content: opening });
  history.push({ role: "user", content: opening });

  // Tool declarations for the model.
  const tools: LlmTool[] = Object.entries(def.tools ?? {}).map(
    ([name, tool]) => ({
      name,
      description: tool.description,
      input_schema: validatorSchemaToJsonSchema(tool.args ?? {}),
    }),
  );

  const maxSteps = def.maxSteps ?? DEFAULT_MAX_STEPS;
  const usage = { input_tokens: 0, output_tokens: 0 };
  let finalText = "";
  let steps = 0;
  let cancelled = false;
  let settled = false;

  const drain = (extra: Record<string, unknown>) =>
    write({ op: "drainInput", steps: priorSteps + steps, ...extra }) as Promise<
      unknown
    > as Promise<DrainResult>;

  try {
    for (;;) {
      steps += 1;
      if (steps > maxSteps) {
        throw ctx.error(
          "AGENT_MAX_STEPS",
          `Agent exceeded ${maxSteps} tool round-trips`,
        );
      }
      const system =
        typeof def.system === "function" ? def.system(ctx, args) : def.system;
      const res = await ctx.llm.stream(
        {
          messages: history,
          ...(system ? { system } : {}),
          ...(tools.length > 0 ? { tools } : {}),
          ...(def.model ? { model: def.model } : {}),
          ...(def.maxTokens ? { max_tokens: def.maxTokens } : {}),
        },
        (e) => {
          if (e.type === "text_delta") ctx.stream.write(e.text);
        },
      );
      usage.input_tokens += res.usage.input_tokens;
      usage.output_tokens += res.usage.output_tokens;

      // Persist the assistant turn exactly as the model produced it
      // (text + tool_use blocks) so history replays are faithful.
      await write({ op: "appendMessage", role: "assistant", content: res.content });
      history.push({ role: "assistant", content: res.content });
      finalText = res.content
        .filter((b): b is Extract<LlmContentBlock, { type: "text" }> => b.type === "text")
        .map((b) => b.text)
        .join("");

      if (res.stop_reason !== "tool_use") {
        // Terminal boundary. Settling and draining in one transaction
        // is what stops a message that lands right now from being
        // stranded on a completed run.
        const pending = await drain({ completeIfEmpty: true });
        // Whatever was queued has already left the row, so persist it
        // either way — cancelling must not silently swallow words the
        // user had typed and can still see queued in the UI.
        const text = pending.input.join("\n\n");
        if (text !== "") {
          await write({ op: "appendMessage", role: "user", content: text });
          history.push({ role: "user", content: text });
        }
        if (pending.cancelRequested) {
          cancelled = true;
          break;
        }
        if (pending.completed) {
          settled = true;
          break;
        }
        // Steered after the model had stopped: the queued text became
        // the next user turn and the same invocation keeps going.
        continue;
      }

      // 3. Execute every requested tool; failures become is_error
      // results the model can react to rather than run-fatal throws.
      const batch = await runToolBatch(def, ctx, runId, res.content);

      // Tool boundary. Every tool_use needs its tool_result even when
      // the batch stopped early, or the transcript can't be replayed.
      const pending = await drain({});
      const stopping = pending.cancelRequested || batch.cancelled;
      const blocks: LlmContentBlock[] = [...batch.results];
      for (const text of pending.input) {
        blocks.push({ type: "text", text });
      }
      await write({ op: "appendMessage", role: "user", content: blocks });
      history.push({ role: "user", content: blocks });
      if (stopping) {
        cancelled = true;
        break;
      }
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    // Best-effort — the failure we surface is the loop's, not the
    // bookkeeping write's.
    await write({
      op: "setStatus",
      status: "failed",
      error: message,
      steps: priorSteps + steps,
    }).catch(() => {});
    throw err;
  }

  if (cancelled) {
    await write({
      op: "setStatus",
      status: "cancelled",
      steps: priorSteps + steps,
    });
    return { runId, text: finalText, steps, usage, cancelled: true };
  }
  // `settled` means drainInput already wrote the terminal status in
  // the same transaction that proved nothing was queued.
  if (!settled) {
    await write({
      op: "setStatus",
      status: "completed",
      steps: priorSteps + steps,
    });
  }
  return { runId, text: finalText, steps, usage };
}

/** Run one turn's tool calls, watching for a cancel while they run.
 *
 *  The watch is a poll rather than a push because a cancel can be
 *  requested on a different machine than the loop runs on — the run
 *  row is the only thing both sides share. It runs only for the
 *  duration of the batch: during `ctx.llm.stream` the host blocks its
 *  per-call read loop, so an RPC issued then would not be answered. */
async function runToolBatch(
  def: AgentDefinition,
  ctx: ActionCtx,
  runId: string,
  turn: LlmContentBlock[],
): Promise<{ results: LlmContentBlock[]; cancelled: boolean }> {
  const calls = turn.filter(
    (b): b is Extract<LlmContentBlock, { type: "tool_use" }> =>
      b.type === "tool_use",
  );
  const results: LlmContentBlock[] = [];
  if (calls.length === 0) return { results, cancelled: false };

  const controller = new AbortController();
  const toolCtx = ctxWithSignal(ctx, controller.signal);
  let cancelled = false;
  let polling = false;
  const timer = setInterval(() => {
    // Skip rather than stack: a slow poll must not queue more.
    if (polling || cancelled) return;
    polling = true;
    void ctx
      .runQuery<{ cancelRequested: boolean }>("__pylon_agent_poll", { runId })
      .then((s) => {
        if (s.cancelRequested && !cancelled) {
          cancelled = true;
          controller.abort(new Error("agent run cancelled"));
        }
      })
      .catch(() => {})
      .finally(() => {
        polling = false;
      });
  }, CANCEL_POLL_MS);

  try {
    for (const block of calls) {
      let content: string;
      let isError = false;
      if (cancelled) {
        // Stop starting new work, but still answer the call so the
        // model sees why it has no result.
        content = "cancelled by user";
        isError = true;
      } else {
        const tool = def.tools?.[block.name];
        if (!tool) {
          content = `Unknown tool "${block.name}"`;
          isError = true;
        } else {
          try {
            if (tool.args) {
              const check = validateArgs(block.input, tool.args);
              if (!check.valid) {
                throw new Error(`Invalid tool input: ${check.errors.join("; ")}`);
              }
            }
            const value = await tool.handler(toolCtx, block.input);
            content =
              typeof value === "string" ? value : JSON.stringify(value ?? null);
          } catch (err) {
            content = err instanceof Error ? err.message : String(err);
            isError = true;
          }
        }
      }
      content = truncateToolResult(content);
      // Announce the tool call on the stream so live UIs can render
      // "using searchDocs…" without polling the message rows.
      ctx.stream.writeEvent(
        "tool",
        JSON.stringify({ name: block.name, input: block.input, isError }),
      );
      results.push({
        type: "tool_result",
        tool_use_id: block.id,
        content,
        ...(isError ? { is_error: true } : {}),
      });
    }
  } finally {
    clearInterval(timer);
  }
  return { results, cancelled };
}

function storedToLlmMessage(m: StoredMessage): LlmMessage {
  const role = m.role === "assistant" ? "assistant" : "user";
  return { role, content: m.content as LlmMessage["content"] };
}
