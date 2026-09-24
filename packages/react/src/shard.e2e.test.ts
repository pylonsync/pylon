// End-to-end: connectShard against a real MessagePack shard.
//
// Runs only when PYLON_SHARD_E2E holds the JSON line printed by
// `cargo run -p pylon-runtime --example shard_codec_server`
// (tools/smoke-shard-codecs.sh sets it up).

import { expect, test } from "bun:test";

import { connectShard } from "./useShard";
import type { ShardInputRejection } from "./shardWire";

const config = process.env.PYLON_SHARD_E2E
  ? (JSON.parse(process.env.PYLON_SHARD_E2E) as {
      port: number;
      shard: string;
      tokens: { ts: string };
    })
  : null;

interface Arena {
  players: Array<{ id: string; x: number; y: number }>;
  label: string;
}

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

test.skipIf(!config)("MessagePack snapshots, binary inputs, acks, and rejections", async () => {
  const client = connectShard<Arena, { dx: number; dy: number }>(config!.shard, {
    subscriberId: "ts",
    token: config!.tokens.ts,
    baseUrl: "127.0.0.1",
    wsPort: config!.port,
    autoReconnect: false,
  });
  let latest: { snapshot: Arena; ack: number } | undefined;
  const rejections: ShardInputRejection[] = [];
  const errors: Error[] = [];
  client.onSnapshot((snapshot, _tick, ack) => {
    latest = { snapshot, ack };
  });
  client.onInputRejected((r) => rejections.push(r));
  client.onError((e) => errors.push(e));

  // The first frame decodes from MessagePack into an object.
  await waitFor("a snapshot", () => latest);
  expect(latest!.snapshot.label).toBe("arena");

  // Inputs now go as binary MessagePack frames (the client learned the codec).
  for (let i = 0; i < 3; i++) client.send({ dx: 1, dy: 2 });
  const moved = await waitFor("ack 3", () => (latest && latest.ack >= 3 ? latest : undefined));
  const me = moved.snapshot.players.find((p) => p.id === "ts");
  expect(me).toEqual({ id: "ts", x: 3, y: 6 });

  // A move apply_input refuses comes back as a rejection frame.
  const seq = client.send({ dx: -999, dy: 0 });
  const rejection = await waitFor("a rejection", () => rejections[0]);
  expect(rejection).toMatchObject({ clientSeq: seq, code: "apply_failed" });
  await waitFor("ack of the rejected input", () =>
    latest && latest.ack >= seq ? latest : undefined,
  );

  expect(errors).toEqual([]);
  client.close();
});
