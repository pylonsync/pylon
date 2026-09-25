// End-to-end: examples/shard-arena on two machines (two `pylon start`
// processes on one Postgres). A shard created on machine B is reached by a
// client that connects to machine A. Killing B starts the shard again on A
// from its saved state, and the client, reconnecting, finds its player where
// it was.
//
// Runs only when PYLON_SHARD_CLUSTER_E2E is "<host:port of A>,<host:port of
// B>" and PYLON_SHARD_CLUSTER_KILL_B is B's process id
// (tools/smoke-shard-cluster.sh sets them).

import { expect, test } from "bun:test";

import { connectShard } from "./connection";

const [hostA, hostB] = (process.env.PYLON_SHARD_CLUSTER_E2E ?? "").split(",");
const pidB = Number(process.env.PYLON_SHARD_CLUSTER_KILL_B ?? 0);

interface Player {
  id: string;
  x: number;
  y: number;
}
interface Arena {
  players: Player[];
}
interface Join {
  shardId: string;
  subscriberId: string;
  ticket: string;
  machine?: string;
}

async function guest(host: string): Promise<string> {
  const res = await fetch(`http://${host}/api/auth/guest`, { method: "POST" });
  expect(res.ok).toBe(true);
  return ((await res.json()) as { token: string }).token;
}

async function join(host: string, token: string, args: Record<string, string>): Promise<Join> {
  const res = await fetch(`http://${host}/api/fn/joinArena`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify(args),
  });
  const text = await res.text();
  if (res.status !== 200) throw new Error(`joinArena on ${host}: ${res.status} ${text}`);
  return JSON.parse(text) as Join;
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

test.skipIf(!hostA || !hostB || !pidB)(
  "a shard on machine B is reached through A, and starts again on A when B dies",
  async () => {
    const arena = `cluster-${Math.random().toString(36).slice(2, 8)}`;
    const token = await guest(hostA);
    // Created through A, placed on B.
    const first = await join(hostA, token, { arena, machine: "b" });
    expect(first.machine).toBe("b");

    let latest: Arena | undefined;
    let opens = 0;
    const errors: string[] = [];
    const client = connectShard<Arena, unknown>(arena, {
      subscriberId: first.subscriberId,
      baseUrl: hostA,
      reconnectBackoffMs: 250,
      // Each connection attempt gets a new ticket from A.
      ticket: async () => (await join(hostA, token, { arena })).ticket,
    });
    client.onSnapshot((snap) => (latest = snap));
    client.onOpen(() => opens++);
    client.onError((e) => errors.push(e.message));

    await waitFor("a snapshot through A", () => latest, 10_000);
    client.send("join");
    client.send({ move_to: { x: 700, y: 400 } });
    const me = () => latest?.players.find((p) => p.id === first.subscriberId);
    await waitFor("the player at its target", () => (me()?.x === 700 && me()?.y === 400 ? true : undefined), 10_000);

    // The directory still says B runs it.
    const onB = await join(hostB, await guest(hostB), { arena });
    expect(onB.machine).toBe("b");

    // Let B save the arena (PYLON_SHARD_SAVE_SECS=1), then kill it.
    await new Promise((r) => setTimeout(r, 2500));
    process.kill(pidB, "SIGKILL");
    latest = undefined;

    // A notices B is dead (no heartbeat for 10 s), starts the arena from
    // its saved state, and the client reconnects to it.
    await waitFor("the arena back on A", () => latest, 40_000);
    const back = me();
    expect(back).toMatchObject({ x: 700, y: 400 });
    expect(opens).toBeGreaterThanOrEqual(2);
    const again = await join(hostA, token, { arena });
    expect(again.machine).toBe("a");
    client.close();
  },
  90_000,
);
