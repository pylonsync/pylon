/**
 * agent() unit tests: the validator→JSON-Schema converter and the tool
 * loop driven against a scripted mock ctx (no Bun runner, no LLM).
 */
import { describe, expect, test } from "bun:test";
import {
  agent,
  isAgentDefinition,
  validatorSchemaToJsonSchema,
} from "./agent";
import { v } from "./validators";
import type {
  ActionCtx,
  LlmCompleteRequest,
  LlmCompleteResponse,
  LlmStreamEvent,
} from "./types";

// ---------------------------------------------------------------------------
// validator → JSON Schema
// ---------------------------------------------------------------------------

describe("validatorSchemaToJsonSchema", () => {
  test("maps every validator shape losslessly", () => {
    const schema = validatorSchemaToJsonSchema({
      q: v.string(),
      n: v.int(),
      score: v.number(),
      on: v.boolean(),
      target: v.id("Doc"),
      tags: v.array(v.string()),
      opts: v.object({ deep: v.optional(v.boolean()) }),
      kind: v.union(v.literal("a"), v.literal("b")),
      blob: v.json(),
      maybe: v.optional(v.string()),
    });
    expect(schema).toEqual({
      type: "object",
      properties: {
        q: { type: "string" },
        n: { type: "integer" },
        score: { type: "number" },
        on: { type: "boolean" },
        target: { type: "string" },
        tags: { type: "array", items: { type: "string" } },
        opts: {
          type: "object",
          properties: { deep: { type: "boolean" } },
        },
        kind: { anyOf: [{ const: "a" }, { const: "b" }] },
        blob: {},
        maybe: { type: "string" },
      },
      required: ["q", "n", "score", "on", "target", "tags", "opts", "kind", "blob"],
    });
  });

  test("empty schema produces an object schema with no required", () => {
    expect(validatorSchemaToJsonSchema({})).toEqual({
      type: "object",
      properties: {},
    });
  });
});

// ---------------------------------------------------------------------------
// Mock ctx harness
// ---------------------------------------------------------------------------

interface WriteOp {
  op: string;
  [key: string]: unknown;
}

interface ReadOverride {
  run?: Record<string, unknown>;
  messages?: Array<Record<string, unknown>>;
}

function mockCtx(script: LlmCompleteResponse[], read?: ReadOverride) {
  const writes: WriteOp[] = [];
  const streamed: Array<{ event?: string; data: string }> = [];
  let call = 0;
  const ctx = {
    auth: { userId: "u1", isAdmin: false, tenantId: null, roles: [] },
    stream: {
      id: "st_test",
      write(data: string) {
        streamed.push({ data });
      },
      writeEvent(event: string, data: string) {
        streamed.push({ event, data });
      },
    },
    llm: {
      async stream(
        _req: LlmCompleteRequest,
        onEvent: (e: LlmStreamEvent) => void,
      ): Promise<LlmCompleteResponse> {
        const res = script[call];
        call += 1;
        if (!res) throw new Error("mock LLM script exhausted");
        for (const block of res.content) {
          if (block.type === "text") {
            onEvent({ type: "text_delta", text: block.text });
          }
        }
        onEvent({ type: "done", stop_reason: res.stop_reason, usage: res.usage });
        return res;
      },
      async complete() {
        throw new Error("not used");
      },
      async embed() {
        throw new Error("not used");
      },
    },
    async runQuery(name: string, args: Record<string, unknown>) {
      if (name === "__pylon_agent_read") {
        return {
          run: {
            id: String(args.runId),
            agent: "helper",
            status: "completed",
            ...read?.run,
          },
          messages: read?.messages ?? [
            { id: "m1", seq: 1, role: "user", content: "earlier question" },
            {
              id: "m2",
              seq: 2,
              role: "assistant",
              content: [{ type: "text", text: "earlier answer" }],
            },
          ],
        };
      }
      throw new Error(`unexpected query ${name}`);
    },
    async runMutation(name: string, args: Record<string, unknown>) {
      if (name !== "__pylon_agent_write") throw new Error(`unexpected ${name}`);
      writes.push(args as WriteOp);
      if (args.op === "createRun") return { id: "run_1" };
      if (args.op === "appendMessage") return { id: "m", seq: writes.length };
      return { updated: true };
    },
    error(code: string, message: string) {
      const err = new Error(message);
      (err as { code?: string }).code = code;
      return err;
    },
  };
  return { ctx: ctx as unknown as ActionCtx, writes, streamed };
}

const done = (text: string): LlmCompleteResponse => ({
  model: "m",
  content: [{ type: "text", text }],
  stop_reason: "end_turn",
  usage: { input_tokens: 10, output_tokens: 5 },
});

const wantsTool = (
  name: string,
  input: Record<string, unknown>,
): LlmCompleteResponse => ({
  model: "m",
  content: [
    { type: "text", text: "Let me check." },
    { type: "tool_use", id: "tu_1", name, input },
  ],
  stop_reason: "tool_use",
  usage: { input_tokens: 20, output_tokens: 8 },
});

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

describe("agent loop", () => {
  test("agent() produces a tagged action with the standard args", () => {
    const def = agent({ system: "hi" });
    expect((def as unknown as Record<string, unknown>).type).toBe("action");
    expect((def as unknown as Record<string, unknown>).auth).toBe("user");
    expect(isAgentDefinition(def)).toBe(true);
    expect(isAgentDefinition(agent({}))).toBe(true);
    expect(isAgentDefinition({ type: "action" })).toBe(false);
  });

  test("new run: tool round-trip, transcript order, terminal status", async () => {
    const toolCalls: unknown[] = [];
    const def = agent({
      system: "You are helpful.",
      tools: {
        lookup: {
          description: "Look something up",
          args: { q: v.string() },
          handler: async (_ctx, input) => {
            toolCalls.push(input);
            return { hits: 3 };
          },
        },
      },
    });
    const { ctx, writes, streamed } = mockCtx([
      wantsTool("lookup", { q: "pylon" }),
      done("Found 3 hits."),
    ]);

    const result = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(ctx, {
      input: "search pylon",
      __agentName: "helper",
    });

    expect(result.runId).toBe("run_1");
    expect(result.text).toBe("Found 3 hits.");
    expect(result.steps).toBe(2);
    expect(result.usage).toEqual({ input_tokens: 30, output_tokens: 13 });
    expect(toolCalls).toEqual([{ q: "pylon" }]);

    // Write sequence: createRun, CLAIM (running, guarded, +streamId),
    // user msg, assistant(tool_use), tool_result, assistant(final),
    // completed. The claim precedes every message write so a losing
    // racer never persists a stray turn.
    expect(writes.map((w) => `${w.op}:${w.role ?? w.status ?? ""}`)).toEqual([
      "createRun:",
      "setStatus:running",
      "appendMessage:user",
      "appendMessage:assistant",
      "appendMessage:user", // tool_result batch
      "appendMessage:assistant",
      "setStatus:completed",
    ]);
    const running = writes.find((w) => w.status === "running")!;
    expect(running.streamId).toBe("st_test");
    expect(running.guardNotRunning).toBe(true);
    expect(Number(running.staleMs)).toBeGreaterThan(0);
    const toolResultMsg = writes[4];
    const blocks = toolResultMsg.content as Array<Record<string, unknown>>;
    expect(blocks[0].type).toBe("tool_result");
    expect(blocks[0].tool_use_id).toBe("tu_1");
    expect(JSON.parse(String(blocks[0].content))).toEqual({ hits: 3 });

    // Streaming: text deltas + a typed tool event.
    expect(streamed.some((s) => s.data === "Let me check.")).toBe(true);
    const toolEvent = streamed.find((s) => s.event === "tool")!;
    expect(JSON.parse(toolEvent.data)).toEqual({
      name: "lookup",
      input: { q: "pylon" },
      isError: false,
    });
  });

  test("invalid tool input and unknown tools become is_error results", async () => {
    const def = agent({
      tools: {
        strict: {
          description: "needs a number",
          args: { n: v.int() },
          handler: async () => "never reached",
        },
      },
    });
    const { ctx, writes } = mockCtx([
      {
        model: "m",
        content: [
          { type: "tool_use", id: "tu_a", name: "strict", input: { n: "NaN" } },
          { type: "tool_use", id: "tu_b", name: "ghost", input: {} },
        ],
        stop_reason: "tool_use",
        usage: { input_tokens: 1, output_tokens: 1 },
      },
      done("Recovered."),
    ]);

    const result = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(ctx, {
      input: "go",
      __agentName: "helper",
    });
    expect(result.text).toBe("Recovered.");

    const toolResults = writes.find(
      (w) =>
        w.op === "appendMessage" &&
        Array.isArray(w.content) &&
        (w.content as Array<Record<string, unknown>>).some(
          (b) => b.type === "tool_result",
        ),
    )!;
    const blocks = (toolResults.content as Array<Record<string, unknown>>).filter(
      (b) => b.type === "tool_result",
    );
    expect(blocks).toHaveLength(2);
    expect(blocks.every((b) => b.is_error === true)).toBe(true);
    expect(String(blocks[1].content)).toContain('Unknown tool "ghost"');
  });

  test("continuation loads history and rejects agent mismatch", async () => {
    const def = agent({});
    const { ctx, writes } = mockCtx([done("With context.")]);
    const result = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(ctx, {
      input: "follow-up",
      runId: "run_9",
      __agentName: "helper",
    });
    expect(result.runId).toBe("run_9");
    // No createRun for continuations.
    expect(writes.some((w) => w.op === "createRun")).toBe(false);

    // Wrong agent name → AGENT_MISMATCH.
    const other = mockCtx([done("nope")]);
    await expect(
      (def.handler as unknown as (
        c: ActionCtx,
        a: Record<string, unknown>,
      ) => Promise<unknown>)(other.ctx, {
        input: "x",
        runId: "run_9",
        __agentName: "differentAgent",
      }),
    ).rejects.toThrow(/belongs to agent/);
  });

  test("a fresh running run is RUN_BUSY; a stale one is taken over", async () => {
    const def = agent({ timeout: 600 });
    const busy = mockCtx([done("never")], {
      run: { status: "running", updatedAt: new Date().toISOString() },
    });
    await expect(
      (def.handler as unknown as (
        c: ActionCtx,
        a: Record<string, unknown>,
      ) => Promise<unknown>)(busy.ctx, {
        input: "x",
        runId: "run_9",
        __agentName: "helper",
      }),
    ).rejects.toThrow(/already generating/);
    expect(busy.writes).toHaveLength(0);

    // updatedAt older than the timeout window → dead generation,
    // continuation proceeds.
    const stale = mockCtx([done("Took over.")], {
      run: {
        status: "running",
        updatedAt: new Date(Date.now() - 700_000).toISOString(),
      },
    });
    const result = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(stale.ctx, {
      input: "continue",
      runId: "run_9",
      __agentName: "helper",
    });
    expect(result.text).toBe("Took over.");
  });

  test("dangling tool_use in history is repaired with error results", async () => {
    const def = agent({});
    const { ctx, writes } = mockCtx([done("Recovered context.")], {
      messages: [
        { id: "m1", seq: 1, role: "user", content: "do the thing" },
        {
          id: "m2",
          seq: 2,
          role: "assistant",
          content: [
            { type: "text", text: "On it." },
            { type: "tool_use", id: "tu_dead", name: "lookup", input: {} },
          ],
        },
      ],
    });
    await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<unknown>)(ctx, {
      input: "still there?",
      runId: "run_9",
      __agentName: "helper",
    });
    // The repair batch persists BEFORE the new user turn.
    const repair = writes[1];
    expect(repair.op).toBe("appendMessage");
    const blocks = repair.content as Array<Record<string, unknown>>;
    expect(blocks[0].type).toBe("tool_result");
    expect(blocks[0].tool_use_id).toBe("tu_dead");
    expect(blocks[0].is_error).toBe(true);
    expect(String(blocks[0].content)).toContain("interrupted");
    expect(writes[2].content).toBe("still there?");
  });

  test("oversized tool results are truncated before persisting", async () => {
    const def = agent({
      tools: {
        dump: {
          description: "returns a lot",
          handler: async () => "x".repeat(80 * 1024),
        },
      },
    });
    const { ctx, writes } = mockCtx([
      wantsTool("dump", {}),
      done("Done."),
    ]);
    await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<unknown>)(ctx, { input: "go", __agentName: "helper" });
    const toolResults = writes.find(
      (w) =>
        w.op === "appendMessage" &&
        Array.isArray(w.content) &&
        (w.content as Array<Record<string, unknown>>).some(
          (b) => b.type === "tool_result",
        ),
    )!;
    const block = (toolResults.content as Array<Record<string, unknown>>)[0];
    const content = String(block.content);
    expect(content.length).toBeLessThan(70 * 1024);
    expect(content.endsWith("[truncated]")).toBe(true);
  });

  test("maxSteps overrun fails the run", async () => {
    const def = agent({
      maxSteps: 1,
      tools: {
        loop: {
          description: "always called",
          handler: async () => "again",
        },
      },
    });
    // The model asks for a tool every time — exceeds maxSteps=1.
    const { ctx, writes } = mockCtx([
      wantsTool("loop", {}),
      wantsTool("loop", {}),
    ]);
    await expect(
      (def.handler as unknown as (
        c: ActionCtx,
        a: Record<string, unknown>,
      ) => Promise<unknown>)(ctx, { input: "go", __agentName: "helper" }),
    ).rejects.toThrow(/tool round-trips/);
    const failed = writes.find((w) => w.status === "failed")!;
    expect(String(failed.error)).toContain("tool round-trips");
  });

  test("llm failure marks the run failed and rethrows", async () => {
    const def = agent({});
    const { ctx, writes } = mockCtx([]); // script exhausted immediately
    await expect(
      (def.handler as unknown as (
        c: ActionCtx,
        a: Record<string, unknown>,
      ) => Promise<unknown>)(ctx, { input: "go", __agentName: "helper" }),
    ).rejects.toThrow(/script exhausted/);
    expect(writes.some((w) => w.status === "failed")).toBe(true);
  });
});
