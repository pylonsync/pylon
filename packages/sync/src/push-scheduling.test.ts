import { afterEach, expect, test } from "bun:test";
import { createTestEnv, type TestEnv } from "./test-harness";
import type { PendingMutation } from "./mutation-queue";

let env: TestEnv | undefined;
afterEach(async () => { await env?.dispose(); env = undefined; });

async function start() {
  env = createTestEnv({ transport: "poll" });
  env.signIn({ userId: "u1" });
  await env.start();
  return env;
}

test("concurrent offline writes share one queue snapshot and request", async () => {
  const { engine, server } = await start();
  const pending = engine.mutations.pending.bind(engine.mutations);
  let copies = 0;
  engine.mutations.pending = () => {
    const rows = pending();
    copies += rows.length;
    return rows;
  };
  server.primeNextPushOutcome({ kind: "network" });
  await Promise.all(Array.from({ length: 1000 }, (_, i) => engine.insert("Note", { title: String(i) })));
  expect(server.pushRequestCount).toBe(1);
  expect(copies).toBe(1000);
  expect(pending()).toHaveLength(1000);
  expect(engine.store.list("Note")).toHaveLength(1000);
});

test("new offline writes retain one retry and explicit recovery sends the queue", async () => {
  const { engine, server } = await start();
  const internals = engine as unknown as { retryTimers: Set<unknown>; pushRetryTimer: unknown };
  server.primeNextPushOutcome({ kind: "network" });
  await engine.insert("Note", { title: "first" });
  const timer = internals.pushRetryTimer;
  for (let i = 1; i < 100; i++) await engine.insert("Note", { title: String(i) });
  expect(server.pushRequestCount).toBe(1);
  expect(internals.pushRetryTimer).toBe(timer);
  expect(internals.retryTimers.size).toBe(1);
  expect(engine.mutations.pending()).toHaveLength(100);
  await engine.push();
  expect(server.pushRequestCount).toBe(2);
  expect(engine.mutations.pending()).toHaveLength(0);
  expect(internals.pushRetryTimer).toBeNull();
  expect(internals.retryTimers.size).toBe(0);
});

test("writes added during a request drain after it completes", async () => {
  const { engine, server } = await start();
  const fetch = globalThis.fetch;
  let release!: () => void;
  let entered!: () => void;
  const started = new Promise<void>((resolve) => { entered = resolve; });
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let first = true;
  globalThis.fetch = (async (input, init) => {
    if (String(input).endsWith("/api/sync/push") && first) {
      first = false;
      entered();
      await gate;
    }
    return fetch(input, init);
  }) as typeof fetch;
  const a = engine.insert("Note", { title: "first" });
  await started;
  const b = engine.insert("Note", { title: "second" });
  release();
  await Promise.all([a, b]);
  expect(server.pushRequestCount).toBe(2);
  expect(engine.mutations.pending()).toHaveLength(0);
  expect(server.receivedPushKeys).toHaveLength(2);
});

test("follower writes forward only new entries and explicit replay forwards all", async () => {
  const { engine } = await start();
  const internals = engine as unknown as {
    isMultiTabLeader: boolean;
    broadcastToTabs(payload: { type: string; ops?: PendingMutation[] }): void;
  };
  internals.isMultiTabLeader = false;
  const batches: number[] = [];
  internals.broadcastToTabs = (payload) => {
    if (payload.type !== "mutations") return;
    batches.push(payload.ops!.length);
    for (const op of payload.ops!) engine.mutations.settleQueued(op.id);
  };
  for (let i = 0; i < 100; i++) await engine.insert("Note", { title: String(i) });
  expect(batches).toEqual(Array(100).fill(1));
  expect(engine.mutations.pending()).toHaveLength(100);
  await engine.push();
  expect(batches.at(-1)).toBe(100);
});

test("stop cancels the pending push retry", async () => {
  const { engine, server } = await start();
  server.primeNextPushOutcome({ kind: "network" });
  await engine.insert("Note", { title: "offline" });
  engine.stop();
  const internals = engine as unknown as { retryTimers: Set<unknown>; pushRetryTimer: unknown };
  expect(internals.pushRetryTimer).toBeNull();
  expect(internals.retryTimers.size).toBe(0);
});

test("explicit replay during a request includes writes added directly to the queue", async () => {
  const { engine, server } = await start();
  const fetch = globalThis.fetch;
  let release!: () => void;
  let entered!: () => void;
  const started = new Promise<void>((resolve) => { entered = resolve; });
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let first = true;
  globalThis.fetch = (async (input, init) => {
    if (String(input).endsWith("/api/sync/push") && first) {
      first = false;
      entered();
      await gate;
    }
    return fetch(input, init);
  }) as typeof fetch;
  const a = engine.insert("Note", { title: "first" });
  await started;
  engine.mutations.add({ entity: "Note", row_id: "direct", kind: "insert", data: { id: "direct" } }, undefined, "u1");
  const replay = engine.push();
  release();
  await Promise.all([a, replay]);
  expect(server.pushRequestCount).toBe(2);
  expect(server.receivedPushKeys).toContain("Note/direct");
  expect(engine.mutations.pending()).toHaveLength(0);
});

test("writes added during a failed request wait for the same retry", async () => {
  const { engine, server } = await start();
  const fetch = globalThis.fetch;
  let release!: () => void;
  let entered!: () => void;
  const started = new Promise<void>((resolve) => { entered = resolve; });
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let first = true;
  globalThis.fetch = (async (input, init) => {
    if (String(input).endsWith("/api/sync/push") && first) {
      first = false;
      entered();
      await gate;
    }
    return fetch(input, init);
  }) as typeof fetch;
  server.primeNextPushOutcome({ kind: "network" });
  const a = engine.insert("Note", { title: "first" });
  await started;
  const b = engine.insert("Note", { title: "second" });
  release();
  await Promise.all([a, b]);
  expect(server.pushRequestCount).toBe(1);
  expect(engine.mutations.pending()).toHaveLength(2);
  await engine.push();
  expect(server.pushRequestCount).toBe(2);
  expect(engine.mutations.pending()).toHaveLength(0);
});

test("identity reset clears the previous queue's retry delay", async () => {
  const { engine, server } = await start();
  server.primeNextPushOutcome({ kind: "network" });
  await engine.insert("Note", { title: "old" });
  await engine.resetReplica({ wipeMutations: true });
  await engine.insert("Note", { title: "new" });
  expect(server.pushRequestCount).toBe(2);
  expect(engine.mutations.pending()).toHaveLength(0);
  expect(engine.store.list("Note")).toHaveLength(1);
});
