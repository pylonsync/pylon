// `refreshSessionIfStale` (what useSession calls on mount) asks
// /api/auth/me at most once per session resolve: a screen that mounts many
// session readers at once must not send one request each.

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

/** Count /api/auth/me requests made through the harness's fetch. */
function countAuthMe(): () => number {
  let n = 0;
  const inner = globalThis.fetch;
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    if (url.endsWith("/api/auth/me")) n++;
    return inner(input, init);
  }) as typeof fetch;
  restoreFetch = () => {
    globalThis.fetch = inner;
  };
  return () => n;
}

test("readers that mount during start join its request", async () => {
  env = await createTestEnv({ transport: "poll" });
  env.signIn({ userId: "u1" });
  const count = countAuthMe();
  const started = env.engine.start();
  const mounts = Array.from({ length: 8 }, () => env!.engine.refreshSessionIfStale());
  await Promise.all([started, ...mounts]);
  expect(env.engine.resolvedSession().userId).toBe("u1");
  expect(count()).toBe(1);
});

test("readers that mount after the session resolved send nothing", async () => {
  env = await createTestEnv({ transport: "poll" });
  env.signIn({ userId: "u1" });
  await env.engine.start();
  const count = countAuthMe();
  await Promise.all(Array.from({ length: 8 }, () => env!.engine.refreshSessionIfStale()));
  expect(count()).toBe(0);
});

test("a new token is resolved once, however many readers mount", async () => {
  env = await createTestEnv({ transport: "poll" });
  env.signIn({ userId: "u1" });
  await env.engine.start();
  const count = countAuthMe();
  env.signIn({ userId: "u2" });
  await Promise.all(Array.from({ length: 8 }, () => env!.engine.refreshSessionIfStale()));
  expect(env.engine.resolvedSession().userId).toBe("u2");
  expect(count()).toBe(1);
  // Resolved for the new token: no more requests.
  await env.engine.refreshSessionIfStale();
  expect(count()).toBe(1);
});

test("notifySessionChanged still always asks the server", async () => {
  env = await createTestEnv({ transport: "poll" });
  env.signIn({ userId: "u1" });
  await env.engine.start();
  const count = countAuthMe();
  await env.engine.notifySessionChanged();
  await env.engine.notifySessionChanged();
  expect(count()).toBe(2);
});
