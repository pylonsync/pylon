// End-to-end: examples/shard-arena on a real `pylon start`. Its shard logic
// is Rust compiled to WebAssembly and loaded by the stock binary.
//
// Runs only when PYLON_WASM_SHARD_E2E holds the server's host:port
// (tools/smoke-wasm-shard.sh sets it up).

import { afterAll, beforeAll, expect, test } from "bun:test";

import { connectShard } from "./useShard";
import type { ShardInputRejection } from "./shardWire";

const host = process.env.PYLON_WASM_SHARD_E2E ?? "";

// The test preload installs happy-dom, whose fetch enforces CORS against the
// page URL. Put the page on the server's origin for these tests.
const happyDOM = (globalThis as { happyDOM?: { setURL(url: string): void } }).happyDOM;
let previousUrl = "";
beforeAll(() => {
  if (!host || !happyDOM) return;
  previousUrl = location.href;
  happyDOM.setURL(`http://${host}/`);
});
afterAll(() => {
  if (previousUrl && happyDOM) happyDOM.setURL(previousUrl);
});

interface Player {
  id: string;
  x: number;
  y: number;
  tx: number;
  ty: number;
  hue: number;
}

interface Arena {
  width: number;
  height: number;
  players: Player[];
}

type Input = "join" | { move_to: { x: number; y: number } };

interface Join {
  shardId: string;
  subscriberId: string;
  ticket: string;
}

async function guestJoin(): Promise<Join> {
  const guest = await fetch(`http://${host}/api/auth/guest`, { method: "POST" });
  expect(guest.ok).toBe(true);
  const { token } = (await guest.json()) as { token: string };
  const res = await fetch(`http://${host}/api/fn/joinArena`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: "{}",
  });
  expect(res.status).toBe(200);
  return (await res.json()) as Join;
}

function waitFor<T>(what: string, check: () => T | undefined, ms = 8000): Promise<T> {
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
    }, 20);
  });
}

function open(join: Join, ticket = join.ticket) {
  const client = connectShard<Arena, Input>(join.shardId, {
    subscriberId: join.subscriberId,
    ticket,
    baseUrl: host,
    autoReconnect: false,
  });
  const state: {
    latest?: Arena;
    ack: number;
    rejections: ShardInputRejection[];
    errors: Error[];
    opened: boolean;
    closed: boolean;
  } = { ack: 0, rejections: [], errors: [], opened: false, closed: false };
  client.onSnapshot((snapshot, _tick, ack) => {
    state.latest = snapshot;
    state.ack = ack;
  });
  client.onInputRejected((r) => state.rejections.push(r));
  client.onError((e) => state.errors.push(e));
  client.onOpen(() => (state.opened = true));
  client.onClose(() => (state.closed = true));
  return { client, state };
}

test.skipIf(!host)("two players move in a WebAssembly shard over /shard", async () => {
  const [a, b] = await Promise.all([guestJoin(), guestJoin()]);
  expect(a.shardId).toBe(b.shardId);
  const pa = open(a);
  const pb = open(b);

  await waitFor("both snapshots", () => (pa.state.latest && pb.state.latest ? true : undefined));
  // MessagePack snapshots decode into objects.
  expect(pa.state.latest!.width).toBe(800);

  pa.client.send("join");
  pb.client.send("join");
  const target = { x: 700, y: 400 };
  const seq = pa.client.send({ move_to: target });

  // B sees A arrive at the target: the module's tick moves the dot.
  const seenByB = await waitFor("A at its target, seen by B", () =>
    pb.state.latest?.players.find(
      (p) => p.id === a.subscriberId && p.x === target.x && p.y === target.y,
    ),
  );
  expect(seenByB.tx).toBe(target.x);
  expect(pa.state.ack).toBeGreaterThanOrEqual(seq);
  expect(pb.state.latest!.players.some((p) => p.id === b.subscriberId)).toBe(true);

  // The module refuses a move outside the arena; the refusal comes back.
  const bad = pa.client.send({ move_to: { x: 5000, y: 0 } });
  const rejection = await waitFor("a rejection", () => pa.state.rejections[0]);
  expect(rejection).toMatchObject({ clientSeq: bad, code: "apply_failed" });
  expect(rejection.message).toContain("outside the arena");

  expect(pa.state.errors).toEqual([]);
  expect(pb.state.errors).toEqual([]);
  pa.client.close();
  pb.client.close();
});

test.skipIf(!host)("a ticket for another subscriber is refused", async () => {
  const [a, b] = await Promise.all([guestJoin(), guestJoin()]);
  // B's ticket names B; presenting it as A fails the handshake.
  const stolen = open(a, b.ticket);
  await waitFor("the refused connection to close", () =>
    stolen.state.closed || stolen.state.errors.length > 0 ? true : undefined,
  );
  expect(stolen.state.latest).toBeUndefined();
  stolen.client.close();
});
