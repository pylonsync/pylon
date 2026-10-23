// Pull pages and far-behind catch-up.
//
//   - Pages grow: 1000 rows, then double per full page up to 10,000, so a
//     large first sync costs a few round trips instead of hundreds.
//   - A far-behind cursor is answered with a snapshot of current rows
//     (`snapshot_ok=1`). The replica is not wiped: rows stay on screen, the
//     snapshot's rows apply in place, rows it no longer holds are deleted
//     (writes not yet on the server are kept), and the log resumes from the
//     snapshot's seq.

import { afterEach, expect, test } from "bun:test";
import { createTestEnv, type TestEnv } from "./test-harness";

let env: TestEnv | null = null;
afterEach(async () => {
  if (env) {
    await env.dispose();
    env = null;
  }
});

const limits = (urls: string[]) =>
  urls.map((u) => Number(new URL(u, "http://test").searchParams.get("limit")));

test("a first sync of 25,000 rows takes 5 pages, growing from 1000", async () => {
  env = await createTestEnv({ transport: "poll" });
  env.server.snapshotMode = true;
  env.signIn({ userId: "u1" });
  env.server.seed(
    "Property",
    Array.from({ length: 25_000 }, (_, i) => ({ id: `p${String(i).padStart(5, "0")}`, n: i })),
  );
  await env.engine.start();
  await env.engine.pull();
  expect(env.engine.store.list("Property").length).toBe(25_000);
  const pulls = env.server.pullUrls.filter((u) => u.includes("since=0") || u.includes("snapshot_after"));
  expect(limits(pulls).slice(0, 5)).toEqual([1000, 2000, 4000, 8000, 10000]);
  expect(pulls.length).toBe(5);
});

test("a far-behind client applies a snapshot in place and resumes the log", async () => {
  env = await createTestEnv({ transport: "poll" });
  env.server.snapshotMode = true;
  env.signIn({ userId: "u1" });
  env.server.seed("Lead", [
    { id: "a", name: "A" },
    { id: "b", name: "B" },
    { id: "c", name: "C" },
  ]);
  await env.engine.start();
  await env.engine.pull();
  expect(env.engine.store.list("Lead").map((r) => r.id).sort()).toEqual(["a", "b", "c"]);

  // While the client is away: b is deleted, c renamed, d added.
  env.server.delete("Lead", "b");
  env.server.update("Lead", "c", { name: "C2" });
  env.server.insert("Lead", { id: "d", name: "D" });
  env.server.farBehind = () => true;

  // Watch for the replica going empty mid-catch-up (a wipe).
  let emptied = false;
  const unsubscribe = env.engine.store.subscribe(() => {
    if (env!.engine.store.list("Lead").length === 0) emptied = true;
  });
  const before = env.server.pullUrls.length;
  await env.engine.pull();
  unsubscribe();

  const rows = env.engine.store.list("Lead");
  expect(rows.map((r) => r.id).sort()).toEqual(["a", "c", "d"]);
  expect(rows.find((r) => r.id === "c")?.name).toBe("C2");
  expect(emptied).toBe(false);
  const catchUp = env.server.pullUrls.slice(before);
  expect(catchUp[0]).toContain("snapshot_ok=1");

  // The log resumes from the snapshot's seq: the next pull is a delta.
  env.server.farBehind = () => false;
  env.server.insert("Lead", { id: "e", name: "E" });
  await env.engine.pull();
  expect(env.engine.store.list("Lead").map((r) => r.id).sort()).toEqual(["a", "c", "d", "e"]);
  const last = env.server.pullUrls[env.server.pullUrls.length - 1]!;
  expect(last).not.toContain("since=0");
});

test("the sweep keeps rows with writes the server has not seen", async () => {
  env = await createTestEnv({ transport: "poll" });
  env.server.snapshotMode = true;
  env.signIn({ userId: "u1" });
  env.server.seed("Lead", [{ id: "a", name: "A" }]);
  await env.engine.start();
  await env.engine.pull();

  // Offline write: queued, not on the server.
  env.server.primeNextPushOutcome({ kind: "network" });
  const localId = await env.engine.insert("Lead", { name: "made offline" });
  env.server.farBehind = () => true;
  await env.engine.pull();
  const ids = env.engine.store.list("Lead").map((r) => r.id);
  expect(ids).toContain("a");
  expect(ids).toContain(localId);
});
