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
  /** Max model↔tool round-trips per invocation (default 16). Hitting
   *  the cap fails the run rather than looping forever. */
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
  /** The user's message for this turn. */
  input: string;
  /** Continue an existing run (must belong to the caller and this
   *  agent). Omit to start a new run. */
  runId?: string;
  /** Optional display title, stored on new runs. */
  title?: string;
}

/** What the agent action resolves with (also the `event: result`
 *  payload on the SSE stream). */
export interface AgentResult {
  runId: string;
  /** Concatenated text of the final assistant message. */
  text: string;
  /** Round-trips consumed. */
  steps: number;
  usage: { input_tokens: number; output_tokens: number };
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
      input: v.string(),
      runId: v.optional(v.string()),
      title: v.optional(v.string()),
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

const DEFAULT_MAX_STEPS = 16;

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

  // A run left "running" longer than the agent's timeout is a dead
  // generation (the process died before the terminal status write) —
  // continuations may take it over instead of being RUN_BUSY forever.
  const staleMs = Math.max(def.timeout ?? 600, 60) * 1000;

  // 1. Create or load the run (ownership enforced inside the internal
  // fns, which run under this caller's auth).
  let runId: string;
  let history: LlmMessage[] = [];
  if (args.runId) {
    const loaded = await ctx.runQuery<{
      run: { id: string; agent: string; status: string; updatedAt?: string };
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
        throw ctx.error(
          "RUN_BUSY",
          "This run is already generating — wait for it to finish",
        );
      }
    }
    runId = loaded.run.id;
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
  // Repair with synthetic error results before the new user turn.
  const lastMsg = history[history.length - 1];
  if (lastMsg?.role === "assistant" && Array.isArray(lastMsg.content)) {
    const dangling = lastMsg.content.filter(
      (b): b is Extract<LlmContentBlock, { type: "tool_use" }> =>
        (b as { type?: string }).type === "tool_use",
    );
    if (dangling.length > 0) {
      const repairs: LlmContentBlock[] = dangling.map((b) => ({
        type: "tool_result",
        tool_use_id: b.id,
        content: "tool execution was interrupted",
        is_error: true,
      }));
      await write({ op: "appendMessage", role: "user", content: repairs });
      history.push({ role: "user", content: repairs });
    }
  }

  await write({ op: "appendMessage", role: "user", content: args.input });
  history.push({ role: "user", content: args.input });

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

      if (res.stop_reason !== "tool_use") break;

      // 3. Execute every requested tool; failures become is_error
      // results the model can react to rather than run-fatal throws.
      const results: LlmContentBlock[] = [];
      for (const block of res.content) {
        if (block.type !== "tool_use") continue;
        const tool = def.tools?.[block.name];
        let content: string;
        let isError = false;
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
            const value = await tool.handler(ctx, block.input);
            content =
              typeof value === "string" ? value : JSON.stringify(value ?? null);
          } catch (err) {
            content = err instanceof Error ? err.message : String(err);
            isError = true;
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
      await write({ op: "appendMessage", role: "user", content: results });
      history.push({ role: "user", content: results });
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    // Best-effort — the failure we surface is the loop's, not the
    // bookkeeping write's.
    await write({ op: "setStatus", status: "failed", error: message }).catch(
      () => {},
    );
    throw err;
  }

  await write({ op: "setStatus", status: "completed" });
  return { runId, text: finalText, steps, usage };
}

function storedToLlmMessage(m: StoredMessage): LlmMessage {
  const role = m.role === "assistant" ? "assistant" : "user";
  return { role, content: m.content as LlmMessage["content"] };
}
