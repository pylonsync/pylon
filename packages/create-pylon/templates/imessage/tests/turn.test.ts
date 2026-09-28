// The agent turn: processTurn runs the assistant as the owner through
// ctx.agents.run, and the tools find their contact from the run's context.
import { describe, expect, test } from "bun:test";
import processTurn from "../functions/processTurn";
import resolveTurn from "../functions/resolveTurn";
import { chunkReply } from "../lib/chunk";
import { currentTurn } from "../lib/turn";
import { appBaseUrl } from "../lib/urls";
import { fakeCtx } from "./fake-ctx";

type Handler = (ctx: unknown, args: Record<string, unknown>) => Promise<any>;
const run = (def: unknown, ctx: unknown, args: Record<string, unknown>) =>
  (def as { handler: Handler }).handler(ctx, args);

const STARTED = {
  state: "started",
  turnId: "turn_1",
  conversationId: "conv_1",
  runId: null,
  input: "[iMessage from Ana] hi",
};

/** An action ctx for processTurn with the agent call stubbed. */
function turnCtx(agentsRun: (name: string, options: Record<string, unknown>) => Promise<unknown>) {
  const finished: Record<string, unknown>[] = [];
  const agentCalls: { name: string; options: Record<string, unknown> }[] = [];
  const ctx = {
    env: { ANTHROPIC_API_KEY: "sk-test" },
    async runMutation(name: string, args: Record<string, unknown>) {
      if (name === "beginTurn") return STARTED;
      if (name === "finishTurn") {
        finished.push(args);
        return {};
      }
      throw new Error(`unexpected mutation ${name}`);
    },
    async runQuery(name: string) {
      if (name === "ownerAccount") return { userId: "owner_1" };
      throw new Error(`unexpected query ${name}`);
    },
    agents: {
      run(name: string, options: Record<string, unknown>) {
        agentCalls.push({ name, options });
        return agentsRun(name, options);
      },
    },
  };
  return { ctx, finished, agentCalls };
}

describe("processTurn", () => {
  test("runs the assistant as the owner with the turn as context", async () => {
    const reply = "Hello Ana.\n\nSee you soon.";
    const t = turnCtx(async () => ({ runId: "run_1", text: reply, steps: 1 }));
    const out = await run(processTurn, t.ctx, { conversationId: "conv_1" });
    expect(out).toEqual({ state: "answered", bubbles: chunkReply(reply).length });
    expect(t.agentCalls).toEqual([
      {
        name: "assistant",
        options: {
          input: STARTED.input,
          as: { userId: "owner_1", admin: true },
          context: { conversationId: "conv_1", turnId: "turn_1" },
        },
      },
    ]);
    expect(t.finished).toEqual([
      {
        conversationId: "conv_1",
        turnId: "turn_1",
        outcome: "answered",
        runId: "run_1",
        chunks: chunkReply(reply),
      },
    ]);
  });

  test("records an agent failure on the turn instead of throwing", async () => {
    const t = turnCtx(async () => {
      throw Object.assign(new Error("Upstream LLM provider returned an error."), { code: "PROVIDER_HTTP_529" });
    });
    const out = await run(processTurn, t.ctx, { conversationId: "conv_1" });
    expect(out).toEqual({ state: "failed" });
    expect(t.finished[0]).toMatchObject({
      outcome: "failed",
      detail: "PROVIDER_HTTP_529: Upstream LLM provider returned an error.",
    });
  });
});

describe("the tool fence", () => {
  test("resolveTurn maps a running turn to its contact and nothing else", async () => {
    const f = fakeCtx();
    const contactId = await f.ctx.db.insert("Contact", { handle: "+15125550148" });
    const conversationId = await f.ctx.db.insert("Conversation", {
      contactId,
      turnState: "running",
      turnId: "turn_1",
    });
    expect(await run(resolveTurn, f.ctx, { conversationId, turnId: "turn_1" })).toEqual({ conversationId, contactId });
    // An older turn of the same conversation.
    expect(await run(resolveTurn, f.ctx, { conversationId, turnId: "turn_0" })).toBeNull();
    // The turn finished.
    await f.ctx.db.update("Conversation", conversationId, { turnState: "idle" });
    expect(await run(resolveTurn, f.ctx, { conversationId, turnId: "turn_1" })).toBeNull();
    expect(await run(resolveTurn, f.ctx, { conversationId: "missing", turnId: "turn_1" })).toBeNull();
  });

  test("currentTurn needs the server-set context", async () => {
    const queries: unknown[] = [];
    const ctx = {
      async runQuery(name: string, args: unknown) {
        queries.push([name, args]);
        return { conversationId: "conv_1", contactId: "contact_1" };
      },
    } as never;
    const base = { runId: "run_1", agent: "assistant", userId: "owner_1" };

    expect(
      await currentTurn(ctx, { ...base, context: { conversationId: "conv_1", turnId: "turn_1" } }),
    ).toEqual({ conversationId: "conv_1", contactId: "contact_1" });
    expect(queries).toEqual([["resolveTurn", { conversationId: "conv_1", turnId: "turn_1" }]]);

    // A run a client started (no context), or a partial one, gets nothing.
    await expect(currentTurn(ctx, { ...base, context: null })).rejects.toThrow("only available while answering");
    await expect(currentTurn(ctx, { ...base, context: { conversationId: "conv_1" } })).rejects.toThrow(
      "only available while answering",
    );
    expect(queries.length).toBe(1);
  });
});

describe("appBaseUrl", () => {
  test("prefers the configured public origin, then the local dev server", () => {
    expect(appBaseUrl({ APP_URL: "https://a.example.com/path", PYLON_PORT: "4541" })).toBe("https://a.example.com");
    expect(appBaseUrl({ PYLON_PUBLIC_URL: "https://b.example.com" })).toBe("https://b.example.com");
    expect(appBaseUrl({ PYLON_PORT: "4541" })).toBe("http://127.0.0.1:4541");
    expect(appBaseUrl({ APP_URL: "javascript:alert(1)" })).toBe("http://127.0.0.1:4321");
    expect(appBaseUrl({})).toBe("http://127.0.0.1:4321");
  });
});
