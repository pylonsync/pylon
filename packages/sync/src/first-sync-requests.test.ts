// A first sync is one snapshot. Measured on the Veritas build: after a
// 12-page snapshot, the client fetched every entity a second time through
// /api/entities/<E>/cursor (28 requests, 6.5 MB), because the connect
// reconcile ran after a later delta pull reset the "started from zero"
// flag, and empty observed entities each started their own reconcile.

import { afterEach, expect, test } from "bun:test";
import { createTestEnv, type TestEnv } from "./test-harness";

let env: TestEnv | null = null;
let restoreFetch: (() => void) | null = null;
afterEach(async () => {
  restoreFetch?.();
  restoreFetch = null;
  if (env) {
    await env.dispose();
    env = null;
  }
});

function recordUrls(): string[] {
  const urls: string[] = [];
  const inner = globalThis.fetch;
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    urls.push(typeof input === "string" ? input : input instanceof URL ? input.href : input.url);
    return inner(input, init);
  }) as typeof fetch;
  restoreFetch = () => {
    globalThis.fetch = inner;
  };
  return urls;
}

test("a cold start with a websocket makes no cursor requests after the snapshot", async () => {
  env = await createTestEnv({ transport: "websocket" });
  env.server.snapshotMode = true;
  env.signIn({ userId: "u1" });
  env.server.seed("Property", Array.from({ length: 50 }, (_, i) => ({ id: `p${i}` })));
  env.server.seed("Visit", Array.from({ length: 20 }, (_, i) => ({ id: `v${i}` })));
  const urls = recordUrls();
  // Screens observe entities before and during the first sync, including
  // ones with no rows.
  env.engine.observeEntity("Property");
  env.engine.observeEntity("Member");
  await env.engine.start();
  env.engine.observeEntity("Task");
  await env.flush();
  await env.engine.pull();
  await env.flush();

  expect(env.engine.store.list("Property").length).toBe(50);
  expect(env.engine.store.list("Visit").length).toBe(20);
  expect(urls.filter((u) => u.includes("/cursor"))).toEqual([]);
});

test("a reconnect within the interval skips the sweep; after it, the sweep runs", async () => {
  env = await createTestEnv({ transport: "websocket", autoReconcileIntervalMs: 50, reconnectDelay: 5 });
  env.server.snapshotMode = true;
  env.signIn({ userId: "u1" });
  env.server.seed("Property", [{ id: "p1" }]);
  await env.engine.start();
  await env.flush();
  const urls = recordUrls();

  env.transport.closeLatestWs();
  await env.flush();
  expect(urls.filter((u) => u.includes("/cursor"))).toEqual([]);

  await new Promise((r) => setTimeout(r, 60));
  env.transport.closeLatestWs();
  await env.flush();
  await env.flush(100);
  expect(urls.some((u) => u.includes("/api/entities/Property/cursor"))).toBe(true);
});
