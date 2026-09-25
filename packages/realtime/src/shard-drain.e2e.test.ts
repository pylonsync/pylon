// End-to-end: a deploy does not disconnect players (issue #34). A zone runs
// on machine D; a player there has hit points, a buff, and a position. D
// gets SIGTERM, as in a rolling deploy: it hands the zone to another
// machine with its state and tells the client, which reconnects at once.
// The player sees no error and no transfer, and its state is the same.
//
// Runs only when PYLON_SHARD_DRAIN_E2E is "<host:port of A>,<host:port of
// D>" and PYLON_SHARD_DRAIN_PID is D's pid (tools/smoke-shard-cluster.sh
// sets both). Machine D has PYLON_REPLICA_ID d and saves shards only when
// it stops (PYLON_SHARD_SAVE_SECS=3600).

import { expect, test } from "bun:test";

import { connectShard } from "./connection";

const [hostA, hostD] = (process.env.PYLON_SHARD_DRAIN_E2E ?? "").split(",");
const pidD = Number(process.env.PYLON_SHARD_DRAIN_PID ?? "");

interface Player {
  x: number;
  hp: number;
  loaded: boolean;
  buffs: Array<{ name: string; remaining_ms: number }>;
}

interface Zone {
  zone: string;
  players: Record<string, Player>;
}

type Joined = { subscriberId: string; ticket: string; machine?: string };

async function guest(host: string): Promise<string> {
  const res = await fetch(`http://${host}/api/auth/guest`, { method: "POST" });
  expect(res.ok).toBe(true);
  return ((await res.json()) as { token: string }).token;
}

async function call<T>(host: string, token: string, fn: string, args: unknown): Promise<T> {
  const res = await fetch(`http://${host}/api/fn/${fn}`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify(args),
  });
  const text = await res.text();
  if (res.status !== 200) throw new Error(`${fn} on ${host}: ${res.status} ${text}`);
  return JSON.parse(text) as T;
}

async function waitFor<T>(what: string, check: () => T | undefined, ms: number): Promise<T> {
  const start = Date.now();
  for (;;) {
    const v = check();
    if (v !== undefined) return v;
    if (Date.now() - start > ms) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 25));
  }
}

test.skipIf(!hostA || !hostD || !pidD)(
  "a machine that shuts down hands its zone to another with the player's state, and the client follows with no error",
  async () => {
    const zone = `drain-${Math.random().toString(36).slice(2, 8)}`;
    const token = await guest(hostA);
    const joined = await call<Joined>(hostA, token, "joinZone", { zone, machine: "d" });
    expect(joined.machine).toBe("d");
    const me = joined.subscriberId;

    let latest: Zone | undefined;
    let opens = 0;
    const errors: Error[] = [];
    const transfers: string[] = [];
    const client = connectShard<Zone, unknown>(zone, {
      subscriberId: me,
      baseUrl: hostA,
      ticket: async (shard) => (await call<Joined>(hostA, token, "joinZone", { zone: shard })).ticket,
    });
    client.onSnapshot((s) => (latest = s));
    client.onOpen(() => (opens += 1));
    // A snapshot from before a reconnect says nothing about the new one.
    client.onClose(() => (latest = undefined));
    client.onError((e) => errors.push(e));
    client.onTransfer((to) => transfers.push(to));
    const mine = () => latest?.players[me];

    await waitFor("the zone's first snapshot", () => latest, 10_000);
    client.send("join");
    await waitFor("the character to load", () => (mine()?.loaded ? true : undefined), 10_000);
    client.send({ buff: { name: "haste", ms: 600_000 } });
    client.send({ hit: { damage: 30 } });
    client.send({ move: { dx: 3 } });
    await waitFor(
      "the player's state",
      () => (mine()?.hp === 70 && mine()?.x === 3 && mine()?.buffs.length ? true : undefined),
      10_000,
    );
    // D saves only at shutdown (PYLON_SHARD_SAVE_SECS is an hour there):
    // the final save is what carries this state.

    const openedBefore = opens;
    const killedAt = Date.now();
    process.kill(pidD, "SIGTERM");

    // The client reconnects, to the zone on another machine.
    await waitFor("the reconnect", () => (opens > openedBefore ? true : undefined), 15_000);
    const after = await waitFor(
      "the player in the moved zone",
      () => (opens > openedBefore && mine()?.loaded ? mine() : undefined),
      15_000,
    );
    expect(Date.now() - killedAt).toBeLessThan(10_000);
    expect(after.hp).toBe(70);
    expect(after.x).toBe(3);
    expect(after.buffs.map((b) => b.name)).toEqual(["haste"]);
    expect(errors).toEqual([]);
    expect(transfers).toEqual([]);
    expect(client.shardId).toBe(zone);
    const now = await call<Joined>(hostA, token, "joinZone", { zone });
    expect(now.machine).not.toBe("d");

    // Inputs work on the new machine.
    client.send({ hit: { damage: 1 } });
    await waitFor("an input on the new machine", () => (mine()?.hp === 69 ? true : undefined), 10_000);
    client.close();
  },
  60_000,
);
