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

/** Mutable stand-in for the AgentRun columns the loop steers on, so a
 *  test can simulate a message or a cancel landing mid-run. */
interface RunState {
  pendingInput: string[];
  cancelRequested: boolean;
  status: string;
}

function mockCtx(script: LlmCompleteResponse[], read?: ReadOverride) {
  const writes: WriteOp[] = [];
  const streamed: Array<{ event?: string; data: string }> = [];
  const state: RunState = {
    pendingInput: [],
    cancelRequested: false,
    status: "running",
  };
  let polls = 0;
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
      if (name === "__pylon_agent_poll") {
        polls += 1;
        return {
          status: state.status,
          cancelRequested: state.cancelRequested,
          pending: state.pendingInput.length,
        };
      }
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
      // Mirrors agent-internals' transactional semantics: take the
      // queue, report the cancel flag, and settle only when nothing
      // arrived and no cancel is pending.
      if (args.op === "drainInput") {
        const input = state.pendingInput.slice();
        state.pendingInput = [];
        let completed = false;
        if (
          input.length === 0 &&
          !state.cancelRequested &&
          args.completeIfEmpty === true
        ) {
          state.status = "completed";
          completed = true;
        }
        return { input, cancelRequested: state.cancelRequested, completed };
      }
      if (args.op === "enqueueInput") {
        state.pendingInput.push(String(args.input));
        return { queued: state.pendingInput.length };
      }
      if (args.op === "requestCancel") {
        state.cancelRequested = true;
        return { status: state.status, accepted: true };
      }
      if (args.op === "setStatus") state.status = String(args.status);
      return { updated: true };
    },
    error(code: string, message: string) {
      const err = new Error(message);
      (err as { code?: string }).code = code;
      return err;
    },
  };
  return {
    ctx: ctx as unknown as ActionCtx,
    writes,
    streamed,
    state,
    pollCount: () => polls,
  };
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
    // user msg, assistant(tool_use), the tool boundary's drain,
    // tool_result, assistant(final), then the terminal drain — which
    // settles the run itself, so there is no separate completed write.
    // The claim precedes every message write so a losing racer never
    // persists a stray turn.
    expect(writes.map((w) => `${w.op}:${w.role ?? w.status ?? ""}`)).toEqual([
      "createRun:",
      "setStatus:running",
      "appendMessage:user",
      "appendMessage:assistant",
      "drainInput:",
      "appendMessage:user", // tool_result batch
      "appendMessage:assistant",
      "drainInput:",
    ]);
    const running = writes.find((w) => w.status === "running")!;
    expect(running.streamId).toBe("st_test");
    expect(running.guardNotRunning).toBe(true);
    expect(Number(running.staleMs)).toBeGreaterThan(0);
    // The terminal drain settles the run in the same transaction that
    // proves nothing was queued.
    const terminal = writes[writes.length - 1];
    expect(terminal.completeIfEmpty).toBe(true);
    const toolResultMsg = writes[5];
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

  test("a message to a live run is queued, not refused; a stale one is taken over", async () => {
    const def = agent({ timeout: 600 });
    const busy = mockCtx([done("never")], {
      run: { status: "running", updatedAt: new Date().toISOString() },
    });
    const queued = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(busy.ctx, {
      input: "actually, use the other file",
      runId: "run_9",
      __agentName: "helper",
    });
    // Steering: the message lands on the queue and no turn starts.
    expect(queued.queued).toBe(true);
    expect(queued.runId).toBe("run_9");
    expect(queued.steps).toBe(0);
    expect(busy.state.pendingInput).toEqual(["actually, use the other file"]);
    expect(busy.writes.map((w) => w.op)).toEqual(["enqueueInput"]);
    // No claim, no transcript write — the live generation owns those.
    expect(busy.writes.some((w) => w.op === "setStatus")).toBe(false);

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
    // The repair rides in the SAME user message as the new input.
    // Two user messages in a row are rejected by the Messages API just
    // as a dangling tool_use is, so splitting them would swap one
    // unreplayable transcript for another.
    const repair = writes[1];
    expect(repair.op).toBe("appendMessage");
    expect(repair.role).toBe("user");
    const blocks = repair.content as Array<Record<string, unknown>>;
    expect(blocks[0].type).toBe("tool_result");
    expect(blocks[0].tool_use_id).toBe("tu_dead");
    expect(blocks[0].is_error).toBe(true);
    expect(String(blocks[0].content)).toContain("interrupted");
    expect(blocks[1]).toEqual({ type: "text", text: "still there?" });
    // tool_result blocks come first — the API requires them at the
    // head of the user turn that answers a tool_use.
    expect(blocks.filter((b) => b.type === "tool_result")).toHaveLength(1);
  });

  test("with nothing to repair the opening turn stays a plain string", async () => {
    const def = agent({});
    const { ctx, writes } = mockCtx([done("ok")]);
    await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<unknown>)(ctx, { input: "hello", __agentName: "helper" });
    const opening = writes.find(
      (w) => w.op === "appendMessage" && w.role === "user",
    )!;
    expect(opening.content).toBe("hello");
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

  test("queued input drains at the terminal boundary and extends the turn", async () => {
    const def = agent({});
    const { ctx, writes, state } = mockCtx([
      done("First answer."),
      done("Second answer."),
    ]);
    // The user types while the first response is being produced.
    const originalStream = (ctx as unknown as { llm: { stream: unknown } }).llm
      .stream as (...a: unknown[]) => Promise<unknown>;
    let sent = false;
    (ctx as unknown as { llm: { stream: unknown } }).llm.stream = async (
      ...a: unknown[]
    ) => {
      const res = await originalStream(...a);
      if (!sent) {
        sent = true;
        state.pendingInput.push("wait, also check the tests");
      }
      return res;
    };

    const result = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(ctx, {
      input: "go",
      __agentName: "helper",
    });

    // The run did NOT complete after the first response — the queued
    // message became the next user turn instead.
    expect(result.text).toBe("Second answer.");
    expect(result.steps).toBe(2);
    const userTurns = writes.filter(
      (w) => w.op === "appendMessage" && w.role === "user",
    );
    expect(userTurns.map((w) => w.content)).toEqual([
      "go",
      "wait, also check the tests",
    ]);
    expect(state.pendingInput).toEqual([]);
    expect(state.status).toBe("completed");
  });

  test("queued input merges into the tool_result turn, results first", async () => {
    const def = agent({
      tools: {
        lookup: {
          description: "look up",
          handler: async () => "found",
        },
      },
    });
    const { ctx, writes, state } = mockCtx([
      wantsTool("lookup", { q: "a" }),
      done("Done."),
    ]);
    state.pendingInput.push("focus on the second one");

    await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<unknown>)(ctx, { input: "go", __agentName: "helper" });

    const toolTurn = writes.find(
      (w) =>
        w.op === "appendMessage" &&
        Array.isArray(w.content) &&
        (w.content as Array<Record<string, unknown>>).some(
          (b) => b.type === "tool_result",
        ),
    )!;
    const blocks = toolTurn.content as Array<Record<string, unknown>>;
    // tool_result blocks stay at the head of the turn; the steering
    // text rides along behind them rather than as a second user
    // message, which the Messages API would reject.
    expect(blocks[0].type).toBe("tool_result");
    expect(blocks[blocks.length - 1]).toEqual({
      type: "text",
      text: "focus on the second one",
    });
  });

  test("cancel requested mid-run stops at the boundary and marks the run cancelled", async () => {
    const def = agent({
      tools: {
        slow: {
          description: "slow tool",
          handler: async () => "done anyway",
        },
      },
    });
    const { ctx, writes, state } = mockCtx([
      wantsTool("slow", {}),
      done("never reached"),
    ]);
    state.cancelRequested = true;

    const result = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(ctx, {
      input: "go",
      __agentName: "helper",
    });

    expect(result.cancelled).toBe(true);
    expect(state.status).toBe("cancelled");
    // The tool_use still got its tool_result — a transcript missing
    // one cannot be replayed on the next turn.
    const toolTurn = writes.find(
      (w) =>
        w.op === "appendMessage" &&
        Array.isArray(w.content) &&
        (w.content as Array<Record<string, unknown>>).some(
          (b) => b.type === "tool_result",
        ),
    )!;
    expect(toolTurn).toBeDefined();
    // And the loop stopped instead of asking the model again.
    expect(
      writes.filter((w) => w.op === "appendMessage" && w.role === "assistant"),
    ).toHaveLength(1);
  });

  test("a cancel racing queued input keeps the queued text in the transcript", async () => {
    const def = agent({});
    const { ctx, writes, state } = mockCtx([done("First answer.")]);
    const originalStream = (ctx as unknown as { llm: { stream: unknown } }).llm
      .stream as (...a: unknown[]) => Promise<unknown>;
    (ctx as unknown as { llm: { stream: unknown } }).llm.stream = async (
      ...a: unknown[]
    ) => {
      const res = await originalStream(...a);
      // Both land while the model is producing its answer.
      state.pendingInput.push("one more thing");
      state.cancelRequested = true;
      return res;
    };

    const result = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(ctx, {
      input: "go",
      __agentName: "helper",
    });

    expect(result.cancelled).toBe(true);
    // The drain already took the text off the row, so dropping it here
    // would lose words the user watched themselves type.
    const userTurns = writes.filter(
      (w) => w.op === "appendMessage" && w.role === "user",
    );
    expect(userTurns.map((w) => w.content)).toEqual(["go", "one more thing"]);
    expect(state.status).toBe("cancelled");
  });

  test("cancel: true records the request and starts no turn", async () => {
    const def = agent({});
    const { ctx, writes, state } = mockCtx([]); // no LLM script needed
    const result = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(ctx, {
      runId: "run_9",
      cancel: true,
      __agentName: "helper",
    });
    expect(result.cancelled).toBe(true);
    expect(result.runId).toBe("run_9");
    expect(state.cancelRequested).toBe(true);
    expect(writes.map((w) => w.op)).toEqual(["requestCancel"]);
    // The agent name travels with it so a run can't be stopped
    // through a different agent's action.
    expect(writes[0].agent).toBe("helper");
  });

  test("cancel without a runId, and input without either, are refused", async () => {
    const def = agent({});
    const call = (a: Record<string, unknown>) =>
      (def.handler as unknown as (
        c: ActionCtx,
        a: Record<string, unknown>,
      ) => Promise<unknown>)(mockCtx([]).ctx, { __agentName: "helper", ...a });
    await expect(call({ cancel: true })).rejects.toThrow(/runId/);
    await expect(call({})).rejects.toThrow(/input is required/);
  });

  test("a cancel raised while a tool runs aborts ctx.signal and skips the rest", async () => {
    const seen: string[] = [];
    const def = agent({
      tools: {
        first: {
          description: "first",
          handler: async (toolCtx) => {
            seen.push("first");
            // The cancel lands while this handler is in flight; the
            // poller is what notices, so wait past one interval.
            return await new Promise((resolve) => {
              const timer = setInterval(() => {
                if (toolCtx.signal?.aborted) {
                  clearInterval(timer);
                  resolve("aborted early");
                }
              }, 10);
            });
          },
        },
        second: {
          description: "second",
          handler: async () => {
            seen.push("second");
            return "ran";
          },
        },
      },
    });
    const { ctx, state } = mockCtx([
      {
        model: "m",
        content: [
          { type: "tool_use", id: "tu_1", name: "first", input: {} },
          { type: "tool_use", id: "tu_2", name: "second", input: {} },
        ],
        stop_reason: "tool_use",
        usage: { input_tokens: 1, output_tokens: 1 },
      },
      done("never reached"),
    ]);
    setTimeout(() => {
      state.cancelRequested = true;
    }, 30);

    const result = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(ctx, {
      input: "go",
      __agentName: "helper",
    });

    expect(result.cancelled).toBe(true);
    // The running handler saw the abort, and the queued one never
    // started.
    expect(seen).toEqual(["first"]);
    expect(state.status).toBe("cancelled");
  }, 10_000);

  test("the default step budget is high enough for a real tool loop", async () => {
    // 16 was the old default and is nowhere near enough for an agent
    // that reads files before answering.
    const def = agent({
      tools: { step: { description: "one step", handler: async () => "ok" } },
    });
    const script: LlmCompleteResponse[] = [];
    for (let i = 0; i < 40; i += 1) script.push(wantsTool("step", { i }));
    script.push(done("Finished."));
    const { ctx } = mockCtx(script);
    const result = await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>)(ctx, {
      input: "go",
      __agentName: "helper",
    });
    expect(result.text).toBe("Finished.");
    expect(result.steps).toBe(41);
  });

  test("cumulative steps carry across invocations", async () => {
    const def = agent({});
    const { ctx, writes } = mockCtx([done("Next.")], {
      run: { status: "completed", steps: 7 },
    });
    await (def.handler as unknown as (
      c: ActionCtx,
      a: Record<string, unknown>,
    ) => Promise<unknown>)(ctx, {
      input: "again",
      runId: "run_9",
      __agentName: "helper",
    });
    // This invocation ran one step on top of the stored 7.
    const drains = writes.filter((w) => w.op === "drainInput");
    expect(Number(drains[drains.length - 1].steps)).toBe(8);
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
