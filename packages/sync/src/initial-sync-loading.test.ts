// Pins the "initial-sync settled" signal that `useQuery`'s `loading` gates on.
//
// The bug it guards: `loading` used to drop on `isHydrated()`, which flips true
// the instant the local IndexedDB cache loads — immediate and EMPTY on a cold
// cache (first visit) or right after an org switch wipes the replica. So a UI
// dropped out of "loading" while the rows were still en route from the server,
// flashing its empty state for the seconds until the snapshot landed. The fix
// adds `isInitialSyncSettled()`, true only after the first SERVER pull settles
// (or the cache already had rows, or a fallback deadline) — so callers can hold
// a skeleton through that window.
//
// Hardening: each assertion fails if the signal reverts to settling on local
// hydration (the broken behavior). Not decorative.

import { afterEach, describe, expect, test } from "bun:test";

import { createTestEnv, type TestEnv } from "./test-harness";

describe("initial-sync loading signal", () => {
  let env: TestEnv | null = null;

  afterEach(async () => {
    if (env) {
      await env.dispose();
      env = null;
    }
  });

  test("stays pending until the first server pull settles (no empty-state flash on cold load)", async () => {
    let settledMidPull: boolean | null = null;
    env = createTestEnv({
      beforePull: (_auth, since) => {
        // Captured the instant the server is about to answer the FIRST
        // snapshot pull (since === 0). The engine has already finished local
        // hydration here (so `isHydrated()` is true), but it must NOT be
        // "initial-sync settled" yet — that's what keeps `loading` true.
        if (since === 0 && settledMidPull === null) {
          settledMidPull = env!.engine.isInitialSyncSettled();
        }
      },
    });
    env.signIn({ userId: "u1" });

    // Before start(): definitely not settled.
    expect(env.engine.isInitialSyncSettled()).toBe(false);

    await env.start();

    // The crux: mid-pull the signal was still false (a UI would show a
    // skeleton, not its empty state). If `loading` reverts to gating on
    // `isHydrated()`, this captured value flips to true and the test fails.
    expect(settledMidPull as boolean | null).toBe(false);

    // Once the pull confirms (here: an empty result), it settles — so an empty
    // list now reads as a real "no rows", not "still fetching".
    expect(env.engine.isInitialSyncSettled()).toBe(true);
  });

  test("a replica wipe (org switch / token flip) re-enters the loading state", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();
    expect(env.engine.isInitialSyncSettled()).toBe(true);

    // The org-switch path wipes the replica via resetReplicaInner. The signal
    // must reset so the UI re-shows a skeleton during the re-sync instead of
    // flashing the previous org's (or an empty) list.
    await env.engine.resetReplica();
    expect(env.engine.isInitialSyncSettled()).toBe(false);

    // And it re-settles once the next pull lands.
    await env.flush();
    await env.engine.pull();
    await env.flush();
    expect(env.engine.isInitialSyncSettled()).toBe(true);
  });
});
