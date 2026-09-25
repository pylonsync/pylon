// End-to-end: connectShard against a replicating shard. The client's entity
// table follows the server over a real WebSocket.
//
// Runs only when PYLON_SHARD_REPLICATION_E2E holds the JSON line printed by
// `cargo run -p pylon-runtime --example shard_replication_server`
// (tools/smoke-shard-codecs.sh sets it up).

import { expect, test } from "bun:test";

import { connectShard } from "./useShard";

const config = process.env.PYLON_SHARD_REPLICATION_E2E
  ? (JSON.parse(process.env.PYLON_SHARD_REPLICATION_E2E) as {
      port: number;
      shard: string;
      tokens: { ts: string };
    })
  : null;

function waitFor<T>(what: string, check: () => T | undefined, ms = 5000): Promise<T> {
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
    }, 10);
  });
}

test.skipIf(!config)("the entity table follows spawns, moves, components, and despawns", async () => {
  const client = connectShard<unknown, [number, number]>(config!.shard, {
    subscriberId: "ts",
    token: config!.tokens.ts,
    baseUrl: "127.0.0.1",
    wsPort: config!.port,
    autoReconnect: false,
  });
  const errors: Error[] = [];
  client.onError((e) => errors.push(e));
  let frames = 0;
  let sawFull = false;
  client.onReplication((_table, summary) => {
    frames++;
    sawFull ||= summary.full;
  });

  // The first frame is a full baseline with every visible unit.
  await waitFor("the baseline", () => (client.entities.size === 9 ? true : undefined));
  expect(sawFull).toBe(true);
  // The stealthed unit 0 never reaches this subscriber.
  expect(client.entities.get(0)).toBeUndefined();

  // Units move each tick: positions change and stay near their circle.
  const before = client.entities.get(3)!.x;
  await waitFor("a move", () => (client.entities.get(3)!.x !== before ? true : undefined));
  const u3 = client.entities.get(3)!;
  expect(Math.abs(u3.x - 15)).toBeLessThanOrEqual(1.01);

  // A component change arrives.
  client.send([4, 42]);
  await waitFor("hp 42", () => (client.entities.get(4)?.components.get(1)?.[0] === 42 ? true : undefined));

  // A despawn arrives.
  client.send([5, 0]);
  await waitFor("unit 5 gone", () => (client.entities.get(5) === undefined ? true : undefined));
  expect(client.entities.size).toBe(8);
  expect(frames).toBeGreaterThan(3);
  expect(errors).toEqual([]);
  client.close();
});
