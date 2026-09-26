// End-to-end: shards and the app's data (examples/shard-arena's zone, on
// `pylon start` processes that share one Postgres).
//
// - A zone loads a player's character when the player joins, grants an item
//   through a mutation that runs once per key, and writes x back.
// - A mutation's `ctx.shards.send` reaches the zone after the commit, and
//   not at all when it rolls back.
// - A machine is killed while a grant runs, and another right after a grant
//   commits. Another machine starts the zone from its saved state, the zone
//   sends the grant again under the same key, and the item exists once.
//
// Runs only when PYLON_SHARD_DATA_E2E is "<host:port of A>[,<host:port of
// F>,<host:port of G>]" and PYLON_SHARD_ADMIN_TOKEN is the machines' admin
// token. The kill test also needs F and G and PYLON_SHARD_DATA_KILL, "<pid
// of F>,<pid of G>"; machines F and G have PYLON_REPLICA_ID f and g.
// tools/smoke-shard-cluster.sh runs both tests on Postgres, and
// tools/smoke-wasm-shard.sh the first on SQLite.

import { expect, test } from "bun:test";

import { connectShard } from "./connection";

const [hostA, hostF, hostG] = (process.env.PYLON_SHARD_DATA_E2E ?? "").split(
  ",",
);
const [pidF, pidG] = (process.env.PYLON_SHARD_DATA_KILL ?? "")
  .split(",")
  .map(Number);
const adminToken = process.env.PYLON_SHARD_ADMIN_TOKEN ?? "";
const oneMachine = Boolean(hostA && adminToken);
const killable = Boolean(oneMachine && hostF && hostG && pidF && pidG);

interface Player {
  x: number;
  hp: number;
  loaded: boolean;
  character: string;
  /** Item names by row id. */
  items: Record<string, string>;
}

interface Zone {
  zone: string;
  players: Record<string, Player>;
  /** Grant results applied to grants restored from a save. */
  replayed: number;
}

type Joined = { subscriberId: string; ticket: string; machine?: string };

async function guest(host: string): Promise<string> {
  const res = await fetch(`http://${host}/api/auth/guest`, { method: "POST" });
  expect(res.ok).toBe(true);
  return ((await res.json()) as { token: string }).token;
}

async function call<T>(
  host: string,
  token: string,
  fn: string,
  args: unknown,
): Promise<T> {
  const res = await fetch(`http://${host}/api/fn/${fn}`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(args),
  });
  const text = await res.text();
  if (res.status !== 200)
    throw new Error(`${fn} on ${host}: ${res.status} ${text}`);
  return JSON.parse(text) as T;
}

/** Rows of `entity`, read with the admin token. */
async function rows(entity: string): Promise<Record<string, unknown>[]> {
  const res = await fetch(`http://${hostA}/api/entities/${entity}?limit=1000`, {
    headers: { authorization: `Bearer ${adminToken}` },
  });
  const text = await res.text();
  if (res.status !== 200)
    throw new Error(`list ${entity}: ${res.status} ${text}`);
  const body = JSON.parse(text) as
    Record<string, unknown>[] | { data: Record<string, unknown>[] };
  return Array.isArray(body) ? body : body.data;
}

async function itemsOf(character: string): Promise<string[]> {
  return (await rows("Item"))
    .filter((i) => i.characterId === character)
    .map((i) => String(i.name));
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Check every `every` ms until `check` gives a value. A check that calls
 * the server uses [`HTTP_POLL_MS`]: at 50 ms, one client reaches the
 * function rate limit, and every later check is refused.
 */
async function waitFor<T>(
  what: string,
  check: () => Promise<T | undefined> | T | undefined,
  ms: number,
  every = 50,
): Promise<T> {
  const start = Date.now();
  for (;;) {
    const v = await check();
    if (v !== undefined) return v;
    if (Date.now() - start > ms)
      throw new Error(`timed out waiting for ${what}`);
    await sleep(every);
  }
}

const HTTP_POLL_MS = 500;

/** Join zone `zone` through `host` as a new guest and wait for the character. */
async function joinPlayer(host: string, zone: string, machine?: string, params?: object) {
  const token = await guest(host);
  const joined = await call<Joined>(host, token, "joinZone", { zone, machine, params });
  let snapshot: Zone | undefined;
  const client = connectShard<Zone, unknown>(zone, {
    subscriberId: joined.subscriberId,
    baseUrl: host,
    ticket: async (shard) =>
      (await call<Joined>(host, token, "joinZone", { zone: shard })).ticket,
  });
  client.onSnapshot((s) => (snapshot = s));
  await waitFor(
    "the first snapshot",
    () => (snapshot ? true : undefined),
    10_000,
  );
  client.send("join");
  const me = () => snapshot?.players[joined.subscriberId];
  await waitFor(
    "the character to load",
    () => (me()?.loaded ? true : undefined),
    10_000,
  );
  return {
    token,
    sid: joined.subscriberId,
    client,
    me,
    machine: joined.machine,
  };
}

/** The machine the directory places `zone` on (joinZone reads it, and
 *  starts nothing for a zone that has a placement). */
async function machineOf(token: string, zone: string): Promise<string | undefined> {
  // While the dead machine still holds the placement, the call fails.
  return call<Joined>(hostA, token, "joinZone", { zone }).then(
    (j) => j.machine,
    () => undefined,
  );
}

test.skipIf(!oneMachine)(
  "a zone loads a character, grants an item once, writes x back, and takes a GM's heal after the commit",
  async () => {
    const zone = `data-${Math.random().toString(36).slice(2, 8)}`;
    const p = await joinPlayer(hostA, zone);
    const character = p.me()!.character;
    expect(character).toMatch(/^[0-9a-f]{40}$/);

    p.client.send({ loot: { item: "sword" } });
    await waitFor(
      "the sword",
      () => (Object.values(p.me()?.items ?? {}).includes("sword") ? true : undefined),
      10_000,
    );
    expect(await itemsOf(character)).toEqual(["sword"]);

    // x is written back within the flush interval (2 s).
    p.client.send({ move: { dx: 5 } });
    await waitFor(
      "x in the Character row",
      async () =>
        (await rows("Character")).find((c) => c.id === character)?.x === 5
          ? true
          : undefined,
      10_000,
      HTTP_POLL_MS,
    );

    // A player cannot heal; the zone refuses a heal input from a player too.
    const refused = await call(hostA, p.token, "gmHeal", {
      zone,
      userId: p.sid,
      hp: 50,
    }).catch((e) => e);
    expect(String(refused)).toMatch(/AUTH_REQUIRED|FORBIDDEN/);
    p.client.send({ heal: { who: p.sid, hp: 50 } });

    await call(hostA, adminToken, "gmHeal", { zone, userId: p.sid, hp: 7 });
    await waitFor(
      "the heal",
      () => (p.me()?.hp === 107 ? true : undefined),
      10_000,
    );

    // Rolled back: the zone never gets it, and its log row is gone too.
    const rolledBack = await call(hostA, adminToken, "gmHeal", {
      zone,
      userId: p.sid,
      hp: 1000,
      fail: true,
    }).catch((e) => e);
    expect(String(rolledBack)).toMatch(/GM_TEST_ROLLBACK/);
    await sleep(1000);
    expect(p.me()?.hp).toBe(107);
    expect(
      (await rows("GmAction")).filter((g) => g.zone === zone),
    ).toHaveLength(1);
    p.client.close();
  },
  60_000,
);

test.skipIf(!killable)(
  "a machine killed during a grant, or right after its commit: the item exists once",
  async () => {
    const run = Math.random().toString(36).slice(2, 8);

    // During: the mutation holds its transaction open for 4 s, and F dies
    // in the middle (after the zone saved the grant, every 1 s).
    const during = `grant-during-${run}`;
    const pf = await joinPlayer(hostF, during, "f");
    expect(pf.machine).toBe("f");
    const cf = pf.me()!.character;
    pf.client.send({ loot: { item: "relic", delay_ms: 4000 } });
    await sleep(2000);
    expect(await itemsOf(cf)).toEqual([]);
    process.kill(pidF, "SIGKILL");

    // After: the grant commits, and G dies after a save that still holds
    // the grant as sent (the zone keeps grant results until `release`), so
    // the zone that starts elsewhere must replay the stored result.
    const after = `grant-after-${run}`;
    const pg = await joinPlayer(hostG, after, "g", { hold_results: true });
    expect(pg.machine).toBe("g");
    const cg = pg.me()!.character;
    pg.client.send({ loot: { item: "crown" } });
    await waitFor(
      "the crown's row",
      async () => ((await itemsOf(cg)).length > 0 ? true : undefined),
      10_000,
      HTTP_POLL_MS,
    );
    // Saves every 1 s: at least one after the commit, with the grant.
    await sleep(2500);
    process.kill(pidG, "SIGKILL");

    const watcherToken = await guest(hostA);
    for (const [zone, character, item] of [
      [during, cf, "relic"],
      [after, cg, "crown"],
    ]) {
      await waitFor(
        `${zone} on another machine`,
        async () => {
          const m = await machineOf(watcherToken, zone);
          return m && m !== "f" && m !== "g" ? m : undefined;
        },
        60_000,
        HTTP_POLL_MS,
      );
      await waitFor(
        `the ${item}'s row`,
        async () => ((await itemsOf(character)).length > 0 ? true : undefined),
        20_000,
        HTTP_POLL_MS,
      );
      // Time for any second grant to land.
      await sleep(3000);
      expect(await itemsOf(character)).toEqual([item]);
    }
    pf.client.close();
    pg.client.close();

    // The restarted zones show each item once, after the grant's result;
    // the second one replayed the stored result of its committed grant.
    await expectShown(during, pf.sid, "relic", false);
    await expectShown(after, pg.sid, "crown", true);
  },
  180_000,
);

/**
 * Watch `zone` as a new guest until player `sid` shows `item`, once. With
 * `replay`, release the zone's held grant results until one restored from
 * its save was applied.
 */
async function expectShown(zone: string, sid: string, item: string, replay: boolean) {
  const token = await guest(hostA);
  const t = await call<Joined>(hostA, token, "joinZone", { zone });
  let snapshot: Zone | undefined;
  const watcher = connectShard<Zone, unknown>(zone, {
    subscriberId: t.subscriberId,
    baseUrl: hostA,
    ticket: async (shard) =>
      (await call<Joined>(hostA, token, "joinZone", { zone: shard })).ticket,
  });
  watcher.onSnapshot((s) => (snapshot = s));
  await waitFor(
    `the ${item} in ${zone}`,
    () => (Object.values(snapshot?.players[sid]?.items ?? {}).includes(item) ? true : undefined),
    20_000,
  );
  if (replay) {
    await waitFor(
      `the replayed grant in ${zone}`,
      () => {
        if (snapshot?.replayed === 1) return true;
        watcher.send("release");
        return undefined;
      },
      20_000,
    );
  }
  expect(Object.values(snapshot!.players[sid].items)).toEqual([item]);
  watcher.close();
}
