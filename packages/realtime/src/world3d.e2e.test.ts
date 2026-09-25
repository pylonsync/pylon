// End-to-end: examples/world3d's island shard on a real `pylon start`, with
// two players on connectShardGame: joins, moves drawn by the other player,
// a move the shard clamps and prediction follows, hits, a refused respawn,
// and a leave.
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
  | { join: { x: number; y: number; z: number } }
  | { move: { dx: number; dy: number; dz: number; heading: number; pitch: number } }
  | { hit: { target: number; damage: number } }
  | { spawn: { x: number; y: number; z: number } }
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
  const p: Player = { avatarId: id, game, rejections: [], errors: [], x: 0 };
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
  expect(a.game.send({ join: { x: 0, y: 5, z: 0 } })).toBeGreaterThan(0);
  b.game.send({ join: { x: 20, y: 5, z: 0 } });

  const aEntity = await waitFor("B sees A", () => entityOf(b, a.avatarId));
  const bEntity = await waitFor("A sees B", () => entityOf(a, b.avatarId));
  expect(entityOf(a, a.avatarId)).toBe(aEntity);

  // A walks 10 m at 5 m/s; B draws A in between, then at 10.
  const me = a.game.predict<number>((x, input) =>
    typeof input === "object" && "move" in input ? x + input.move.dx : x,
  );
  let predicted = 0;
  a.game.onReplication((table, _s, _t, ack) => {
    const mine = table.get(aEntity);
    if (mine) predicted = me.reconcile(mine.x, ack);
  });
  const drawn: number[] = [];
  for (let i = 0; i < 40; i++) {
    a.game.send(move(0.25));
    b.game.frame();
    const e = b.game.entities.get(aEntity);
    if (e) drawn.push(e.x);
    await sleep(50);
  }
  const arrived = await waitFor("B draws A at x=10", () => {
    b.game.frame();
    const x = b.game.entities.get(aEntity)?.x;
    return x !== undefined && Math.abs(x - 10) < 0.02 ? x : undefined;
  });
  expect(arrived).toBeCloseTo(10, 1);
  // Drawn between frames: many distinct positions, never backwards.
  expect(new Set(drawn.map((x) => x.toFixed(3))).size).toBeGreaterThan(20);
  for (let i = 1; i < drawn.length; i++) expect(drawn[i]).toBeGreaterThanOrEqual(drawn[i - 1] - 1e-6);
  expect(b.game.clock.tickMs).toBe(50);
  expect(a.game.rttMs).not.toBeNull();

  // A teleport attempt: the shard allows one second of movement (18 m).
  const clampedSeq = a.game.send(move(500));
  await waitFor("the clamped move acknowledged", () => (a.game.ack >= clampedSeq ? true : undefined));
  await waitFor("prediction on the shard's position", () =>
    Math.abs(predicted - (a.game.latest.get(aEntity)?.x ?? NaN)) < 1e-9 ? true : undefined,
  );
  expect(predicted).toBeLessThan(10 + 18.01);
  expect(predicted).toBeGreaterThan(10);

  // A shoots B twice (damage is capped at 60): B is down, and says so.
  a.game.send({ hit: { target: bEntity, damage: 255 } });
  a.game.send({ hit: { target: bEntity, damage: 60 } });
  await waitFor("B dead in B's own table", () =>
    b.game.latest.get(bEntity)?.components.get(HEALTH)?.[0] === 0 ? true : undefined,
  );
  // Too soon to respawn: refused, with the shard's reason.
  const spawnSeq = b.game.send({ spawn: { x: 50, y: 5, z: 50 } });
  const refused = await waitFor("the respawn refused", () =>
    b.rejections.find((r) => r.clientSeq === spawnSeq),
  );
  expect(refused.message).toContain("too soon");

  // A leaves: B draws A until the tick it left, then drops it.
  a.game.send("leave");
  await waitFor("A gone from B's view", () => {
    b.game.frame();
    return b.game.entities.has(aEntity) ? undefined : true;
  });

  expect(a.errors).toEqual([]);
  expect(b.errors).toEqual([]);
  a.game.close();
  b.game.close();
});
