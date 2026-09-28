// Sign-in without a page reload must re-sync EVERY entity.
//
// Repro (crm template): sign in to an existing account in a tab that
// started anonymous. The dashboard showed 24 companies and 0 deals until
// a reload. Two faults combined:
//   1. The session refresh after sign-in (`persistSession` →
//      `notifySessionChanged`) saw a user flip with no tenant flip and
//      did nothing but notify. No reset, no pull, and the live socket
//      stayed bound to the anonymous identity.
//   2. The only rows that arrived came from `observeEntity`'s scoped
//      reconcile, and concurrent scoped reconciles coalesced onto the
//      FIRST caller's entity list, so the second entity never fetched.

import { afterEach, describe, expect, test } from "bun:test";

import { createTestEnv, type TestEnv } from "./test-harness";

// Rows are visible only to a signed-in user.
const signedInOnly = (
  _e: string,
  rows: Array<Record<string, unknown>>,
  auth: { userId: string | null },
) => (auth.userId ? rows : []);

function seedCrm(env: TestEnv) {
  env.server.seed("Company", [
    { id: "c1", name: "Acme" },
    { id: "c2", name: "Globex" },
  ]);
  env.server.seed("Deal", [
    { id: "d1", companyId: "c1", value: 10 },
    { id: "d2", companyId: "c2", value: 20 },
    { id: "d3", companyId: "c2", value: 30 },
  ]);
}

describe("identity change re-syncs every entity", () => {
  let env: TestEnv | null = null;
  afterEach(async () => {
    if (env) await env.dispose();
    env = null;
  });

  test("anonymous → signed-in via notifySessionChanged pulls all entities", async () => {
    env = createTestEnv({ visible: signedInOnly });
    seedCrm(env);
    await env.start();
    expect(env.engine.store.list("Company")).toHaveLength(0);
    expect(env.engine.store.list("Deal")).toHaveLength(0);

    // What persistSession() does: store the token, then refresh.
    env.signIn({ userId: "u1", tenantId: null });
    await env.engine.notifySessionChanged();
    await env.flush(50);

    expect(env.engine.resolvedSession().userId).toBe("u1");
    expect(env.engine.store.list("Company")).toHaveLength(2);
    expect(env.engine.store.list("Deal")).toHaveLength(3);
  });

  test("the live socket reconnects as the signed-in user", async () => {
    env = createTestEnv({ visible: signedInOnly, reconnectDelay: 1 });
    seedCrm(env);
    await env.start();
    await env.flush(50);
    const before = env.transport.wsConnectCount();

    env.signIn({ userId: "u1", tenantId: null });
    await env.engine.notifySessionChanged();
    await env.flush(50);
    expect(env.transport.wsConnectCount()).toBeGreaterThan(before);

    env.server.pushToUser("u1", {
      seq: env.server.nextSeqValue(),
      entity: "Deal",
      row_id: "d-live",
      kind: "insert",
      data: { id: "d-live", value: 99 },
      timestamp: "",
    });
    await env.flush(50);
    expect(env.engine.store.get("Deal", "d-live")).not.toBeNull();
  });

  test("user A → user B through the session refresh wipes A's rows", async () => {
    env = createTestEnv({
      visible: (_e, rows, auth) =>
        rows.filter((r) => (r as { ownerId?: string }).ownerId === auth.userId),
    });
    env.server.seed("Note", [
      { id: "a1", ownerId: "uA" },
      { id: "b1", ownerId: "uB" },
    ]);
    env.signIn({ userId: "uA", tenantId: null });
    await env.start();
    expect(env.engine.store.list("Note").map((r) => r.id)).toEqual(["a1"]);

    env.signIn({ userId: "uB", tenantId: null });
    await env.engine.notifySessionChanged();
    await env.flush(50);
    expect(env.engine.store.list("Note").map((r) => r.id)).toEqual(["b1"]);
  });

  test("concurrent scoped reconciles fetch every requested entity", async () => {
    env = createTestEnv();
    env.signIn({ userId: "u1" });
    await env.start();
    // Rows the engine never saw (created before its cursor).
    seedCrm(env);

    // Two components mount in the same tick and observe their entity.
    const p1 = env.engine.reconcile(["Company"]);
    const p2 = env.engine.reconcile(["Deal"]);
    await Promise.all([p1, p2]);
    await env.flush();

    expect(env.engine.store.list("Company")).toHaveLength(2);
    expect(env.engine.store.list("Deal")).toHaveLength(3);
  });
});
