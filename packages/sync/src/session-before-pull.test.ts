// A sign-in resolves the session as soon as /api/auth/me answers. The
// re-pull of the new identity's rows (seconds on a slow phone) does not
// hold `sessionResolved()` or `useSession` back; the app renders from the
// session and reads progress from `syncProgress()`.

import { afterEach, expect, test } from "bun:test";
import { createTestEnv, type TestEnv } from "./test-harness";

let env: TestEnv | null = null;
afterEach(async () => {
  if (env) {
    await env.dispose();
    env = null;
  }
});

test("the session commits before the new identity's snapshot finishes", async () => {
  let release: () => void = () => {};
  const gate = new Promise<void>((r) => (release = r));
  let holding = false;
  env = await createTestEnv({
    transport: "poll",
    beforePull: async (auth) => {
      if (auth.userId === "u1") {
        holding = true;
        await gate;
      }
    },
  });
  env.server.seed("Lead", [{ id: "a", name: "A" }]);
  await env.engine.start();
  expect(env.engine.resolvedSession().userId).toBeNull();

  env.signIn({ userId: "u1" });
  const refreshed = env.engine.notifySessionChanged();
  // Wait until the re-pull is stuck at the server.
  for (let i = 0; i < 200 && !holding; i++) await new Promise((r) => setTimeout(r, 1));
  expect(holding).toBe(true);
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

  expect(env.engine.sessionResolved()).toBe(true);
  expect(env.engine.resolvedSession().userId).toBe("u1");
  // The old identity's rows are gone; the new ones have not arrived.
  expect(env.engine.store.list("Lead")).toEqual([]);
  expect(env.engine.syncProgress().pulling).toBe(true);

  release();
  await refreshed;
  expect(env.engine.store.list("Lead").map((r) => r.id)).toEqual(["a"]);
  expect(env.engine.syncProgress().pulling).toBe(false);
});
