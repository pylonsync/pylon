// What happens today when two clients edit the same row while offline and
// then reconnect (Plan 013 step 5). This records the behavior; it does not
// change it.
//
// Result: the server applies each update as a field-level patch in the order
// the pushes arrive. Different fields both survive. On the same field the
// last push wins. Both pushes come back "applied", so neither client is told
// that another edit replaced its value: the earlier writer's edit is
// overwritten in silence, and its screen changes when the other edit syncs.

import { afterEach, describe, expect, test } from "bun:test";

import { SyncEngine, type Row } from "./index";
import type { Storage } from "./storage";
import { createTestEnv, type TestEnv } from "./test-harness";

class MemoryStorage implements Storage {
  private map = new Map<string, string>();
  get(key: string) {
    return this.map.get(key) ?? null;
  }
  set(key: string, value: string) {
    this.map.set(key, value);
  }
  remove(key: string) {
    this.map.delete(key);
  }
}

describe("two clients edit one row offline", () => {
  let env: TestEnv | null = null;
  let second: SyncEngine | null = null;
  afterEach(async () => {
    second?.stop();
    second = null;
    if (env) await env.dispose();
    env = null;
  });

  async function twoClients() {
    // Client A is the harness engine; client B is a second engine against
    // the same server, signed in as another rep.
    env = createTestEnv({ transport: "poll", appName: "rep-a" });
    env.signIn({ userId: "rep-a" });
    env.server.seed("Lead", [{ id: "l1", status: "new", note: "", phone: "" }]);
    await env.start();
    const storage = new MemoryStorage();
    storage.set("pylon:rep-b:token", env.server.signIn({ userId: "rep-b" }));
    second = new SyncEngine({
      baseUrl: "http://test.invalid",
      appName: "rep-b",
      persist: false,
      storage,
      transport: "poll",
      multiTab: false,
    });
    await second.start();
    await env.flush();
    return { a: env.engine, b: second, server: env.server };
  }

  /** Let each push's catch-up pull finish, then pull the other's edit
   *  (on the poll transport nothing pushes it). */
  async function syncBoth(a: SyncEngine, b: SyncEngine) {
    await env!.flush(20);
    await a.pull();
    await b.pull();
  }

  async function offlineUpdate(engine: SyncEngine, patch: Row) {
    env!.server.primeNextPushOutcome({ kind: "network" });
    await engine.update("Lead", "l1", patch);
    expect(engine.pendingCount()).toBe(1);
  }

  test("different fields: both edits survive", async () => {
    const { a, b, server } = await twoClients();
    await offlineUpdate(a, { status: "sold" });
    await offlineUpdate(b, { note: "dog in yard" });

    await a.push();
    await b.push();
    await syncBoth(a, b);

    const [row] = await server.listEntityRows("Lead", env!.token);
    expect(row).toMatchObject({ status: "sold", note: "dog in yard" });
    expect(a.store.get("Lead", "l1")).toMatchObject({ status: "sold", note: "dog in yard" });
    expect(b.store.get("Lead", "l1")).toMatchObject({ status: "sold", note: "dog in yard" });
  });

  test("same field: the last push wins and neither client is told", async () => {
    const { a, b, server } = await twoClients();
    await offlineUpdate(a, { status: "sold" });
    await offlineUpdate(b, { status: "not interested" });

    // A reconnects first, then B.
    await a.push();
    await b.push();
    await syncBoth(a, b);

    const [row] = await server.listEntityRows("Lead", env!.token);
    expect(row).toMatchObject({ status: "not interested" });
    // A's edit was replaced: A now shows B's value...
    expect(a.store.get("Lead", "l1")).toMatchObject({ status: "not interested" });
    // ...and nothing reports it. Both writes were applied, so neither
    // outbox has a failed write.
    expect(a.outbox()).toEqual({ pending: [], failed: [] });
    expect(b.outbox()).toEqual({ pending: [], failed: [] });
  });
});
