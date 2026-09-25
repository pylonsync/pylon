// End-to-end: a player moves between shards on two machines
// (examples/shard-arena, two `pylon start` processes on one Postgres), with
// its hit points, buffs, and cooldowns. A move the target refuses leaves the
// player in the source.
//
// Runs only when PYLON_SHARD_TRANSFER_E2E is "<host:port of A>,<host:port of
// B>" (tools/smoke-shard-cluster.sh sets it).

import { expect, test } from "bun:test";

import { connectShard } from "./connection";
import type { ShardInputRejection } from "./wire";

const [hostA, hostB] = (process.env.PYLON_SHARD_TRANSFER_E2E ?? "").split(",");

interface Player {
  x: number;
  hp: number;
  buffs: Array<{ name: string; remaining_ms: number }>;
  cooldowns: Record<string, number>;
}
interface Zone {
  zone: string;
  players: Record<string, Player>;
}

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
  if (res.status !== 200) {
    const err = new Error(`${fn} on ${host}: ${res.status} ${text}`) as Error & { code?: string };
    try {
      err.code = (JSON.parse(text) as { error?: { code?: string } }).error?.code;
    } catch {}
    throw err;
  }
  return JSON.parse(text) as T;
}

function waitFor<T>(what: string, check: () => T | undefined, ms: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const start = Date.now();
    const timer = setInterval(() => {
      const v = check();
      if (v !== undefined) {
        clearInterval(timer);
        resolve(v);
      } else if (Date.now() - start > ms) {
        clearInterval(timer);
        reject(new Error(`timed out waiting for ${what}`));
      }
    }, 25);
  });
}

test.skipIf(!hostA || !hostB)(
  "a player moves to a shard on another machine with its state, and a refused move leaves it",
  async () => {
    const run = Math.random().toString(36).slice(2, 8);
    const [west, east, shut] = [`west-${run}`, `east-${run}`, `shut-${run}`];
    const token = await guest(hostA);
    type Joined = { subscriberId: string; ticket: string; machine?: string };
    const first = await call<Joined>(hostA, token, "joinZone", { zone: west, machine: "a" });
    expect(first.machine).toBe("a");
    expect((await call<Joined>(hostA, token, "joinZone", { zone: east, machine: "b" })).machine).toBe("b");
    const closed = { zone: shut, machine: "b", params: { closed: true } };
    expect((await call<Joined>(hostA, token, "joinZone", closed)).machine).toBe("b");
    const me = first.subscriberId;

    let latest: Zone | undefined;
    const moves: string[] = [];
    const rejections: ShardInputRejection[] = [];
    const client = connectShard<Zone, unknown>(west, {
      subscriberId: me,
      baseUrl: hostA,
      reconnectBackoffMs: 250,
      ticket: async (shard) => (await call<Joined>(hostA, token, "joinZone", { zone: shard })).ticket,
    });
    client.onSnapshot((snap) => (latest = snap));
    client.onTransfer((to) => moves.push(to));
    client.onInputRejected((r) => rejections.push(r));

    await waitFor("a snapshot of the west zone", () => latest, 10_000);
    client.send("join");
    client.send({ buff: { name: "haste", ms: 120_000 } });
    client.send({ cast: { ability: "fireball", cooldown_ms: 120_000 } });
    client.send({ hit: { damage: 30 } });
    client.send({ move: { dx: 2 } });
    const mine = () => latest?.players[me];
    const before = await waitFor(
      "the player's state in west",
      () => (mine()?.hp === 70 && mine()?.x === 2 && mine()?.buffs.length ? mine() : undefined),
      10_000,
    );

    // The closed zone refuses: the player stays in west, still connected.
    const refused = await call(hostA, token, "moveZone", { from: west, to: shut }).catch((e) => e);
    expect((refused as { code?: string }).code).toBe("SHARD_TRANSFER_REFUSED");
    expect(client.shardId).toBe(west);
    client.send({ hit: { damage: 1 } });
    await waitFor("west still has the player", () => (mine()?.hp === 69 ? true : undefined), 10_000);

    // Moved to east, on machine B: the client follows the transfer frame.
    const done = await call<{ shard: string }>(hostA, token, "moveZone", { from: west, to: east });
    expect(done.shard).toBe(east);
    await waitFor("the transfer frame", () => (moves.length ? true : undefined), 10_000);
    expect(moves).toEqual([east]);
    expect(client.shardId).toBe(east);
    const after = await waitFor(
      "the player in east",
      () => (latest?.zone === east ? mine() : undefined),
      10_000,
    );
    expect(after.hp).toBe(69);
    expect(after.x).toBe(2);
    expect(after.buffs.map((b) => b.name)).toEqual(["haste"]);
    const haste = (p: Player) => p.buffs[0].remaining_ms;
    expect(haste(after)).toBeLessThanOrEqual(haste(before));
    expect(haste(after)).toBeGreaterThan(haste(before) - 10_000);
    expect(after.cooldowns.fireball).toBeLessThanOrEqual(before.cooldowns.fireball);
    expect(after.cooldowns.fireball).toBeGreaterThan(before.cooldowns.fireball - 10_000);

    // The cooldown came along: east refuses the cast.
    client.send({ cast: { ability: "fireball", cooldown_ms: 1 } });
    await waitFor(
      "the cooldown refusal",
      () => (rejections.some((r) => r.message.includes("cooling down")) ? true : undefined),
      10_000,
    );
    // West no longer has the player.
    const westNow = await call<Joined>(hostB, await guest(hostB), "joinZone", { zone: west });
    expect(westNow.machine).toBe("a");
    client.close();
  },
  60_000,
);
