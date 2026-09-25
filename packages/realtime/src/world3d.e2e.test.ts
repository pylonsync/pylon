// End-to-end: examples/world3d's island shard on a real `pylon start`, with
// two players on connectShardGame: joins at the shard's spawn points, moves
// drawn by the other player, a move the shard clamps and prediction
// follows, hits and the damage limit, a refused respawn, a leave, and a
// connection without a ticket refused.
//
// Runs only when PYLON_WORLD3D_E2E holds the server's host:port or its
// origin (tools/smoke-wasm-shard.sh sets it up).

import { expect, test } from "bun:test";

import { connectShardGame, type ShardGame } from "./game";
import type { ShardInputRejection } from "./wire";

const target = process.env.PYLON_WORLD3D_E2E ?? "";
const origin = target.includes("://") ? target.replace(/\/$/, "") : `http://${target}`;
const host = target ? new URL(origin).host : "";

type Input =
  | "join"
  | { move: { dx: number; dy: number; dz: number; heading: number; pitch: number } }
  | { hit: { target: number; damage: number } }
  | "spawn"
  | "leave";

const AVATAR = 1;
const HEALTH = 3;

async function call<T>(token: string, fn: string, args: unknown): Promise<T> {
  const res = await fetch(`${origin}/api/fn/${fn}`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify(args),
  });
  const body = await res.text();
  if (res.status !== 200) throw new Error(`${fn}: ${res.status} ${body}`);
  return JSON.parse(body) as T;
}

interface Player {
  avatarId: string;
  token: string;
  userId: string;
  game: ShardGame<Input>;
  rejections: ShardInputRejection[];
  errors: Error[];
  x: number;
}

async function player(): Promise<Player> {
  const guest = await fetch(`${origin}/api/auth/guest`, { method: "POST" });
  expect(guest.ok).toBe(true);
  const { token, user_id } = (await guest.json()) as { token: string; user_id: string };
  const { id } = await call<{ id: string }>(token, "spawnAvatar", { userId: user_id });
  const game = connectShardGame<Input>("island-main", {
    subscriberId: id,
    baseUrl: host,
    tickRate: 20,
    ticket: async () => (await call<{ ticket: string }>(token, "joinIsland", {})).ticket,
  });
  const p: Player = { avatarId: id, token, userId: user_id, game, rejections: [], errors: [], x: 0 };
  game.onInputRejected((r) => p.rejections.push(r));
  game.onError((e) => p.errors.push(e));
  return p;
}

/** The entity whose AVATAR component is `avatarId`, in `p`'s newest table. */
function entityOf(p: Player, avatarId: string): number | undefined {
  for (const e of p.game.latest.entities.values()) {
    const bytes = e.components.get(AVATAR);
    if (bytes && new TextDecoder().decode(bytes) === avatarId) return e.id;
  }
  return undefined;
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

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const move = (dx: number): Input => ({ move: { dx, dy: 0, dz: 0, heading: 0.5, pitch: 0 } });

test.skipIf(!host)("two players on the island shard with connectShardGame", async () => {
  const [a, b] = await Promise.all([player(), player()]);
  await waitFor("both connected", () => (a.game.connected && b.game.connected ? true : undefined));
  expect(a.game.send("join")).toBeGreaterThan(0);
  b.game.send("join");

  const aEntity = await waitFor("B sees A", () => entityOf(b, a.avatarId));
  const bEntity = await waitFor("A sees B", () => entityOf(a, b.avatarId));
  expect(entityOf(a, a.avatarId)).toBe(aEntity);
  // The shard picked the spawn point (shards/island/spawns.json).
  const start = a.game.latest.get(aEntity)!.x;
  expect(Number.isFinite(start)).toBe(true);

  // Prediction: on every frame, the predicted x is the shard's x plus the
  // moves it has not processed.
  const me = a.game.predict<number>((x, input) =>
    typeof input === "object" && "move" in input ? x + input.move.dx : x,
  );
  const sent = new Map<number, number>();
  let predicted = 0;
  let checked = 0;
  a.game.onReplication((table, _s, _t, ack) => {
    const mine = table.get(aEntity);
    if (!mine) return;
    predicted = me.reconcile(mine.x, ack);
    let pending = 0;
    for (const [seq, dx] of sent) if (seq > ack) pending += dx;
    expect(predicted).toBeCloseTo(mine.x + pending, 6);
    checked++;
  });
  const send = (dx: number) => {
    const seq = a.game.send(move(dx));
    sent.set(seq, dx);
    return seq;
  };

  // A walks 10 m at 5 m/s; B draws A in between, then at start + 10.
  const drawn: number[] = [];
  for (let i = 0; i < 40; i++) {
    send(0.25);
    b.game.frame();
    const e = b.game.entities.get(aEntity);
    if (e) drawn.push(e.x);
    await sleep(50);
  }
  const arrived = await waitFor("B draws A 10 m on", () => {
    b.game.frame();
    const x = b.game.entities.get(aEntity)?.x;
    return x !== undefined && Math.abs(x - (start + 10)) < 0.02 ? x : undefined;
  });
  expect(arrived).toBeCloseTo(start + 10, 1);
  // Drawn between frames: many distinct positions, never backwards.
  expect(new Set(drawn.map((x) => x.toFixed(3))).size).toBeGreaterThan(20);
  for (let i = 1; i < drawn.length; i++) expect(drawn[i]).toBeGreaterThanOrEqual(drawn[i - 1] - 1e-6);
  expect(b.game.clock.tickMs).toBe(50);
  expect(a.game.rttMs).not.toBeNull();

  // A teleport attempt: the shard allows its saved budget (18 m), and the
  // prediction follows the shard.
  const clampedSeq = send(500);
  send(0.5);
  await waitFor("the clamped move acknowledged", () => (a.game.ack >= clampedSeq + 1 ? true : undefined));
  const x = a.game.latest.get(aEntity)!.x;
  expect(x - start).toBeGreaterThan(10);
  expect(x - start).toBeLessThan(10 + 18.6);
  expect(predicted).toBeCloseTo(x, 6);
  expect(checked).toBeGreaterThan(20);

  // A shoots B, C, and D twice each at once. Damage is capped at 60 a hit
  // and 300 at once: B and C go down, D takes one hit, and the last is
  // refused.
  const [c, d] = await Promise.all([player(), player()]);
  await waitFor("C and D connected", () => (c.game.connected && d.game.connected ? true : undefined));
  c.game.send("join");
  d.game.send("join");
  const cEntity = await waitFor("A sees C", () => entityOf(a, c.avatarId));
  const dEntity = await waitFor("A sees D", () => entityOf(a, d.avatarId));
  const hits = [bEntity, bEntity, cEntity, cEntity, dEntity, dEntity].map((target) =>
    a.game.send({ hit: { target, damage: 255 } }),
  );
  await waitFor("B dead in B's own table", () =>
    b.game.latest.get(bEntity)?.components.get(HEALTH)?.[0] === 0 ? true : undefined,
  );
  await waitFor("the last hit refused", () =>
    a.rejections.find((r) => r.clientSeq === hits[5] && r.message.includes("faster")),
  );
  await waitFor("D hit once", () =>
    d.game.latest.get(dEntity)?.components.get(HEALTH)?.[0] === 40 ? true : undefined,
  );
  // Too soon to respawn: refused, with the shard's reason.
  const spawnSeq = b.game.send("spawn");
  const refused = await waitFor("the respawn refused", () =>
    b.rejections.find((r) => r.clientSeq === spawnSeq),
  );
  expect(refused.message).toContain("too soon");

  // A leaves: the shard keeps A for 5 s (leaving is no escape from a
  // fight), then B drops it.
  a.game.send("leave");
  await waitFor(
    "A gone from B's view",
    () => {
      b.game.frame();
      return b.game.entities.has(aEntity) ? undefined : true;
    },
    12_000,
  );

  expect(a.errors).toEqual([]);
  expect(b.errors).toEqual([]);
  a.game.close();
  b.game.close();
  c.game.close();
  d.game.close();
  // A leave lingers 5 s before the entity goes.
}, 30_000);

test.skipIf(!host)("a connection without a joinIsland ticket is refused", async () => {
  // The shard runs (a player joined through joinIsland), so the refusal is
  // for the missing ticket.
  const other = await player();
  await waitFor("the shard running", () => (other.game.connected ? true : undefined));
  // A guest session names itself as the subscriber, with no ticket.
  const guest = await fetch(`${origin}/api/auth/guest`, { method: "POST" });
  const { token, user_id } = (await guest.json()) as { token: string; user_id: string };
  const game = connectShardGame<Input>("island-main", {
    subscriberId: user_id,
    baseUrl: host,
    token,
    autoReconnect: false,
  });
  let closed = false;
  game.onClose(() => (closed = true));
  game.onError(() => {});
  await waitFor("the refused connection to close", () => (closed ? true : undefined));
  expect(game.latest.size).toBe(0);
  expect(game.connected).toBe(false);
  game.close();
  other.game.close();
});
