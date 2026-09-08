// `sessionResolved()` tells readers whether `/api/auth/me` has answered.
// Before it does, `resolvedSession()` is the empty placeholder — a route
// guard that reads `userId == null` as "signed out" sends a returning
// user to the sign-in screen on every cold launch. The flag flips on the
// first answer, anonymous or not, and notifies store subscribers.

import { afterEach, expect, test } from "bun:test";
import { SyncEngine } from "./index";
import { createTestEnv, type TestEnv } from "./test-harness";

let env: TestEnv | null = null;

afterEach(async () => {
  if (env) {
    await env.dispose();
    env = null;
  }
});

test("unresolved before start; resolved after /api/auth/me answers anonymous", async () => {
  env = await createTestEnv({ transport: "poll" });
  const { engine } = env;
  expect(engine.sessionResolved()).toBe(false);
  let notified = 0;
  engine.store.subscribe(() => {
    notified++;
  });
  await engine.start();
  expect(engine.sessionResolved()).toBe(true);
  expect(engine.resolvedSession().userId).toBeNull();
  // The placeholder and the anonymous answer are identical, so only the
  // resolved flag changed — subscribers must still hear about it.
  expect(notified).toBeGreaterThan(0);
});

test("resolved with the signed-in identity", async () => {
  env = await createTestEnv({ transport: "poll" });
  env.signIn({ userId: "u1" });
  await env.engine.start();
  expect(env.engine.sessionResolved()).toBe(true);
  expect(env.engine.resolvedSession().userId).toBe("u1");
});

test("an offline start leaves the session unresolved instead of reporting anonymous", async () => {
  // A cold launch with no network: /api/auth/me never answers, so the
  // engine cannot know who the user is. `sessionResolved()` must stay
  // false. A mobile route guard that read the placeholder `userId: null`
  // as "signed out" would send a signed-in user to the sign-in screen
  // every time they open the app on a plane.
  const original = globalThis.fetch;
  globalThis.fetch = (async () => {
    throw new Error("simulated network failure (offline)");
  }) as typeof fetch;
  const engine = new SyncEngine({
    baseUrl: "http://stub.invalid",
    persist: false,
    multiTab: false,
  });
  try {
    await engine.start();
    expect(engine.sessionResolved()).toBe(false);
    expect(engine.resolvedSession().userId).toBeNull();
  } finally {
    engine.stop();
    globalThis.fetch = original;
  }
});
