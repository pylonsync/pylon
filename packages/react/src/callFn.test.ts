// `callFn` routing. The bug this locks down: the exported helper did a bare
// fetch and dropped the `X-Pylon-Change-Seq` response header, so a caller's
// own mutation didn't land in their own replica — add a row, and it isn't
// there until something else refreshes. `db.fn` and `useMutation` both went
// through `SyncEngine.fn`, which observes the header, so the same operation
// behaved differently depending on which import you reached for.

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { callFn } from "./index";
import { peekActiveEngine, setActiveEngine } from "./engine-registry";

describe("callFn", () => {
  const realFetch = globalThis.fetch;
  let requests: Array<{ url: string; headers: Record<string, string> }>;

  beforeEach(() => {
    requests = [];
    setActiveEngine(null);
    globalThis.fetch = (async (input: any, init: any = {}) => {
      requests.push({
        url: String(input?.url ?? input),
        headers: (init.headers ?? {}) as Record<string, string>,
      });
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch;
  });

  afterEach(() => {
    globalThis.fetch = realFetch;
    setActiveEngine(null);
  });

  /** Minimal engine stand-in that records `fn` calls. */
  function fakeEngine() {
    const calls: Array<{ name: string; args: unknown }> = [];
    const engine = {
      async fn<T>(name: string, args: unknown): Promise<T> {
        calls.push({ name, args });
        return { viaEngine: true } as unknown as T;
      },
    };
    setActiveEngine(engine as any);
    return calls;
  }

  test("routes through the engine when the app has one", async () => {
    const calls = fakeEngine();
    const result = await callFn("addRoom", { name: "Main" });
    expect(calls).toEqual([{ name: "addRoom", args: { name: "Main" } }]);
    expect(result).toEqual({ viaEngine: true } as any);
    // The engine owns the request, so nothing should have gone out
    // through the bare transport.
    expect(requests).toHaveLength(0);
  });

  test("falls back to a plain fetch when no engine exists", async () => {
    // Apps that never call init() must keep working — and must NOT have
    // an engine constructed (and a WebSocket opened) by a single POST.
    await callFn("addRoom", { name: "Main" });
    expect(requests).toHaveLength(1);
    expect(requests[0]!.url).toContain("/api/fn/addRoom");
    expect(peekActiveEngine()).toBeNull();
  });

  test("an explicit token bypasses the engine", async () => {
    // That's a call as some other identity. Pulling its results into
    // this replica would mix two users' data.
    const calls = fakeEngine();
    await callFn("addRoom", { name: "Main" }, { token: "someone-elses" });
    expect(calls).toHaveLength(0);
    expect(requests).toHaveLength(1);
  });

  test("peekActiveEngine never constructs an engine", async () => {
    // The distinction from db.ts's getSync(), which deliberately does.
    expect(peekActiveEngine()).toBeNull();
    expect(peekActiveEngine()).toBeNull();
  });
});
