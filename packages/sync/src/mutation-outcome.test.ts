// insert / update / delete report the server's verdict to the caller.
//
// Before: a write the server refused (policy denial, validation) was
// rolled back locally and marked failed, but the promise the caller
// awaited still resolved, so the UI could not say "Message not sent".

import { afterEach, describe, expect, test } from "bun:test";

import { MutationRejectedError, SyncEngine } from "./index";
import type { PendingMutation } from "./mutation-queue";
import { createTestEnv, type TestEnv } from "./test-harness";

describe("mutation outcome", () => {
  let env: TestEnv | null = null;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
  });

  test("insert rejects with the server's error code and removes the ghost", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.denyWrites("Message", "POLICY_DENIED", "not a member of this channel");

    let caught: unknown = null;
    try {
      await env.engine.insert("Message", { body: "hi" });
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(MutationRejectedError);
    const err = caught as MutationRejectedError;
    expect(err.code).toBe("POLICY_DENIED");
    expect(err.message).toBe("not a member of this channel");
    expect(err.entity).toBe("Message");
    expect(err.kind).toBe("insert");
    expect(env.engine.store.list("Message")).toHaveLength(0);
  });

  test("update and delete reject and restore the row", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    env.server.seed("Note", [{ id: "n1", title: "original" }]);
    await env.start();
    env.server.denyWrites("Note", "VALIDATION_FAILED", "title too long");

    await expect(env.engine.update("Note", "n1", { title: "x" })).rejects.toMatchObject({
      name: "MutationRejectedError",
      code: "VALIDATION_FAILED",
      kind: "update",
    });
    expect((env.engine.store.get("Note", "n1") as { title?: string }).title).toBe("original");

    await expect(env.engine.delete("Note", "n1")).rejects.toMatchObject({
      code: "VALIDATION_FAILED",
      kind: "delete",
    });
    expect(env.engine.store.get("Note", "n1")).not.toBeNull();
  });

  test("a whole-request rejection (HTTP 403) rejects the caller", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.primeNextPushOutcome({ kind: "status", status: 403 });
    await expect(env.engine.insert("Note", { title: "x" })).rejects.toMatchObject({
      code: "PUSH_REJECTED",
    });
    expect(env.engine.store.list("Note")).toHaveLength(0);
  });

  test("an applied insert resolves with the row id", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    const id = await env.engine.insert("Note", { title: "ok" });
    expect(env.engine.store.get("Note", id)).not.toBeNull();
    expect(env.server.receivedPushKeys).toContain(`Note/${id}`);
  });

  test("an offline insert resolves (queued) and keeps the ghost", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.primeNextPushOutcome({ kind: "network" });
    const id = await env.engine.insert("Note", { title: "offline" });
    expect(env.engine.store.get("Note", id)).not.toBeNull();
    expect(env.engine.pendingCount()).toBe(1);
  });

  test("a stopped engine does not retry a failed push", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    env.server.primeNextPushOutcome({ kind: "network" });
    await env.engine.insert("Note", { title: "offline" });
    env.engine.stop();
    const before = env.transport.fetchCount();
    // The transient failure scheduled a retry 1s out.
    await new Promise((r) => setTimeout(r, 1300));
    expect(env.transport.fetchCount()).toBe(before);
  });

  test("a write queued while a push is running still reaches the server", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    // The second insert starts while the first push waits on the
    // server. Its push() call joins the running push, whose batch was
    // taken before the second write was queued.
    let release!: () => void;
    env.server.pushGate = new Promise<void>((r) => (release = r));
    const p1 = env.engine.insert("Note", { title: "one" });
    await env.flush(10);
    expect(env.server.pushRequestCount).toBe(1);
    const p2 = env.engine.insert("Note", { title: "two" });
    await env.flush(10);
    env.server.pushGate = null;
    release();
    const [a, b] = await Promise.all([p1, p2]);
    expect(env.server.receivedPushKeys).toContain(`Note/${a}`);
    expect(env.server.receivedPushKeys).toContain(`Note/${b}`);
    expect(env.engine.pendingCount()).toBe(0);
  });

  test("an identity-flip wipe rejects writes still waiting", async () => {
    env = createTestEnv({ transport: "poll" });
    env.signIn({ userId: "u1" });
    await env.start();
    const engine = env.engine as unknown as {
      isMultiTabLeader: boolean;
      broadcastToTabs: (p: unknown) => void;
    };
    // As a follower the write waits for the leader's verdict.
    engine.isMultiTabLeader = false;
    engine.broadcastToTabs = () => {};
    const p = env.engine.insert("Note", { title: "draft" });
    await env.flush();
    await env.engine.resetReplica({ wipeMutations: true });
    await expect(p).rejects.toMatchObject({ code: "DISCARDED" });
  });
});

describe("mutation outcome in a follower tab", () => {
  function follower() {
    const engine = new SyncEngine({
      baseUrl: "http://test.invalid",
      persist: false,
      multiTab: false,
    });
    const sent: unknown[] = [];
    const internals = engine as unknown as {
      isMultiTabLeader: boolean;
      broadcastToTabs: (p: unknown) => void;
      multiTabHooks: () => {
        onMutationsAcked: (ids: string[]) => void;
        onMutationsFailed: (ops: { opId: string; error: string; code?: string }[]) => void;
        onMutationsQueued: (ids: string[]) => void;
      };
    };
    internals.isMultiTabLeader = false;
    internals.broadcastToTabs = (p) => sent.push(p);
    const hooks = internals.multiTabHooks();
    const forwardedOpId = () => {
      const msg = sent.find(
        (m) => (m as { type?: string }).type === "mutations",
      ) as { ops: PendingMutation[] } | undefined;
      return msg!.ops[0]!.id;
    };
    return { engine, hooks, forwardedOpId };
  }

  test("the leader's rejection rejects the follower's caller and rolls back", async () => {
    const { engine, hooks, forwardedOpId } = follower();
    const p = engine.insert("Message", { body: "hi" });
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    expect(engine.store.list("Message")).toHaveLength(1);
    hooks.onMutationsFailed([
      { opId: forwardedOpId(), error: "denied", code: "POLICY_DENIED" },
    ]);
    await expect(p).rejects.toMatchObject({ code: "POLICY_DENIED" });
    expect(engine.store.list("Message")).toHaveLength(0);
  });

  test("the leader's ack resolves the follower's caller", async () => {
    const { engine, hooks, forwardedOpId } = follower();
    const p = engine.insert("Message", { body: "hi" });
    await new Promise((r) => setTimeout(r, 0));
    hooks.onMutationsAcked([forwardedOpId()]);
    expect(typeof (await p)).toBe("string");
  });

  test("the leader's queued notice resolves the follower's caller", async () => {
    const { engine, hooks, forwardedOpId } = follower();
    const p = engine.insert("Message", { body: "hi" });
    await new Promise((r) => setTimeout(r, 0));
    hooks.onMutationsQueued([forwardedOpId()]);
    expect(typeof (await p)).toBe("string");
    expect(engine.store.list("Message")).toHaveLength(1);
  });
});
