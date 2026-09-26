import { beforeEach, describe, expect, test } from "bun:test";
import {
  hasTerminalEvent,
  parseSse,
  readAgentResponse,
  resetOwnerSessionCache,
  runAgentAsOwner,
  selfBaseUrl,
} from "../lib/agent-call";

const RESULT = { runId: "run_1", text: "Hi there", steps: 1, usage: { input_tokens: 1, output_tokens: 1 } };
const SSE_BODY = `retry: 1000\n\nid: 1\ndata: Hi \n\nid: 2\ndata: there\n\nid: 3\nevent: result\ndata: ${JSON.stringify(RESULT)}\n\n`;

beforeEach(() => resetOwnerSessionCache());

describe("SSE parsing", () => {
  test("parses events and multi-line data", () => {
    expect(parseSse("event: tool\ndata: a\ndata: b\n\ndata: plain\n\n")).toEqual([
      { event: "tool", data: "a\nb" },
      { event: "message", data: "plain" },
    ]);
  });

  test("detects a complete terminal event only", () => {
    expect(hasTerminalEvent(SSE_BODY)).toBe(true);
    expect(hasTerminalEvent(SSE_BODY.slice(0, -2))).toBe(false);
    expect(hasTerminalEvent("data: x\n\n")).toBe(false);
  });

  test("reads a streamed result, a streamed error, and a JSON error", () => {
    expect(readAgentResponse(200, "text/event-stream", SSE_BODY)).toEqual({
      ok: true,
      result: { runId: "run_1", text: "Hi there", steps: 1, queued: false, cancelled: false },
    });
    expect(
      readAgentResponse(200, "text/event-stream", `event: error\ndata: {"code":"LLM_NOT_CONFIGURED","message":"no key"}\n\n`),
    ).toEqual({ ok: false, code: "LLM_NOT_CONFIGURED", message: "no key" });
    expect(readAgentResponse(403, "application/json", `{"error":{"code":"FORBIDDEN","message":"admin only"}}`)).toEqual({
      ok: false,
      code: "FORBIDDEN",
      message: "admin only",
    });
  });
});

describe("selfBaseUrl", () => {
  test("prefers loopback, then an https public origin, never plain-http public", () => {
    expect(selfBaseUrl({ PYLON_PORT: "4541", APP_URL: "https://a.example.com" })).toBe("http://127.0.0.1:4541");
    expect(selfBaseUrl({ APP_URL: "https://a.example.com/path" })).toBe("https://a.example.com");
    expect(selfBaseUrl({ PYLON_PUBLIC_URL: "https://b.example.com" })).toBe("https://b.example.com");
    expect(selfBaseUrl({ APP_URL: "http://a.example.com" })).toBe("http://127.0.0.1:4321");
    expect(selfBaseUrl({ APP_URL: "javascript:alert(1)" })).toBe("http://127.0.0.1:4321");
  });
});

/** A streaming response that sends `body` and then never closes. */
function openStream(body: string): Response {
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(body));
    },
  });
  return new Response(stream, { headers: { "content-type": "text/event-stream" } });
}

describe("runAgentAsOwner", () => {
  test("mints an owner session, calls the agent, and returns once the result arrives", async () => {
    const calls: { url: string; auth: string | null; body: unknown }[] = [];
    const fetchImpl = async (url: string, init: RequestInit) => {
      const headers = init.headers as Record<string, string>;
      calls.push({ url, auth: headers.authorization ?? null, body: JSON.parse(String(init.body)) });
      if (url.endsWith("/api/auth/session")) return Response.json({ token: "owner-session", user_id: "u1" }, { status: 201 });
      return openStream(SSE_BODY);
    };
    const env = { APP_URL: "https://app.example.com", PYLON_ADMIN_TOKEN: "admin-token" };
    const out = await runAgentAsOwner(env, "u1", { input: "hi", title: "conv_1" }, fetchImpl);
    expect(out.ok && out.result.text).toBe("Hi there");
    expect(calls.map((c) => c.url)).toEqual([
      "https://app.example.com/api/auth/session",
      "https://app.example.com/api/fn/assistant",
    ]);
    expect(calls[0].auth).toBe("Bearer admin-token");
    expect(calls[0].body).toEqual({ user_id: "u1" });
    expect(calls[1].auth).toBe("Bearer owner-session");
    expect(calls[1].body).toEqual({ input: "hi", title: "conv_1" });

    // The session is reused on the next call.
    await runAgentAsOwner(env, "u1", { input: "again" }, fetchImpl);
    expect(calls.filter((c) => c.url.endsWith("/api/auth/session")).length).toBe(1);
  });

  test("re-mints once when the cached session is rejected", async () => {
    let mints = 0;
    let agentCalls = 0;
    const fetchImpl = async (url: string) => {
      if (url.endsWith("/api/auth/session")) return Response.json({ token: `s${++mints}` });
      agentCalls += 1;
      if (agentCalls === 1) return Response.json({ error: { code: "AUTH_REQUIRED", message: "expired" } }, { status: 401 });
      return openStream(SSE_BODY);
    };
    const out = await runAgentAsOwner({}, "u1", { input: "hi" }, fetchImpl);
    expect(out.ok).toBe(true);
    expect(mints).toBe(2);
  });

  test("reports a refused session mint", async () => {
    const fetchImpl = async () =>
      Response.json({ error: { code: "FORBIDDEN", message: "/api/auth/session requires admin auth" } }, { status: 403 });
    const out = await runAgentAsOwner({}, "u1", { input: "hi" }, fetchImpl);
    expect(out).toEqual({ ok: false, code: "OWNER_SESSION_FORBIDDEN", message: "/api/auth/session requires admin auth" });
  });
});
