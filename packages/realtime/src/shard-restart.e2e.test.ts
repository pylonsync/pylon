// End-to-end: a zone's state survives a restart of a `pylon start` process
// on SQLite (examples/shard-arena's zone). The heal step heals a player,
// which only the zone's state records (the Character row has no hp); the
// check step, after the server restarts, finds the healed hp.
//
// Runs only when PYLON_SHARD_RESTART_E2E is "<host:port>",
// PYLON_SHARD_ADMIN_TOKEN is the server's admin token, PYLON_SHARD_RESTART_FILE
// names a file that carries the zone between steps, and
// PYLON_SHARD_RESTART_STEP is "heal" or "check" with PYLON_SHARD_RESTART_HP
// the hp to expect. tools/smoke-wasm-shard.sh runs it.

import { existsSync, readFileSync, writeFileSync } from "node:fs";

import { expect, test } from "bun:test";

import { connectShard } from "./connection";

const host = process.env.PYLON_SHARD_RESTART_E2E ?? "";
const adminToken = process.env.PYLON_SHARD_ADMIN_TOKEN ?? "";
const file = process.env.PYLON_SHARD_RESTART_FILE ?? "";
const step = process.env.PYLON_SHARD_RESTART_STEP ?? "";
const hp = Number(process.env.PYLON_SHARD_RESTART_HP ?? "0");
const enabled = Boolean(host && adminToken && file && step && hp);

interface Player {
  hp: number;
  loaded: boolean;
}

interface Zone {
  players: Record<string, Player>;
}

type Joined = { subscriberId: string; ticket: string };

/** The zone and the player the steps share. */
interface Carried {
  zone: string;
  sid: string;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function guest(): Promise<string> {
  const res = await fetch(`http://${host}/api/auth/guest`, { method: "POST" });
  expect(res.ok).toBe(true);
  return ((await res.json()) as { token: string }).token;
}

async function call<T>(token: string, fn: string, args: unknown): Promise<T> {
  const res = await fetch(`http://${host}/api/fn/${fn}`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(args),
  });
  const text = await res.text();
  if (res.status !== 200) throw new Error(`${fn}: ${res.status} ${text}`);
  return JSON.parse(text) as T;
}

/**
 * Watch `zone` as a new guest until player `sid` has `want` hp. The zone can
 * still be starting after a restart: each try joins again.
 */
async function waitForHp(zone: string, sid: string, want: number, ms: number) {
  const token = await guest();
  const start = Date.now();
  let seen: number | undefined;
  while (Date.now() - start < ms) {
    let snapshot: Zone | undefined;
    try {
      const joined = await call<Joined>(token, "joinZone", { zone });
      const watcher = connectShard<Zone, unknown>(zone, {
        subscriberId: joined.subscriberId,
        baseUrl: host,
        ticket: async (shard) =>
          (await call<Joined>(token, "joinZone", { zone: shard })).ticket,
      });
      watcher.onSnapshot((s) => (snapshot = s));
      const until = Date.now() + 3000;
      while (Date.now() < until && snapshot?.players[sid]?.hp !== want) {
        await sleep(100);
      }
      watcher.close();
    } catch {
      // The zone is not up yet.
    }
    seen = snapshot?.players[sid]?.hp;
    if (seen === want) return;
    await sleep(500);
  }
  throw new Error(`player ${sid} in ${zone}: hp ${seen}, want ${want}`);
}

test.skipIf(!enabled || step !== "heal")(
  "heal: a player's hp, kept only in the zone's state, goes up",
  async () => {
    let carried: Carried;
    if (existsSync(file)) {
      carried = JSON.parse(readFileSync(file, "utf8")) as Carried;
    } else {
      // A new zone and player.
      const zone = `restart-${Math.random().toString(36).slice(2, 8)}`;
      const token = await guest();
      const joined = await call<Joined>(token, "joinZone", { zone });
      let snapshot: Zone | undefined;
      const client = connectShard<Zone, unknown>(zone, {
        subscriberId: joined.subscriberId,
        baseUrl: host,
        ticket: async (shard) =>
          (await call<Joined>(token, "joinZone", { zone: shard })).ticket,
      });
      client.onSnapshot((s) => (snapshot = s));
      const start = Date.now();
      while (!snapshot) {
        if (Date.now() - start > 10_000) throw new Error("no snapshot");
        await sleep(50);
      }
      client.send("join");
      while (!snapshot?.players[joined.subscriberId]?.loaded) {
        if (Date.now() - start > 10_000) throw new Error("the character did not load");
        await sleep(50);
      }
      client.close();
      carried = { zone, sid: joined.subscriberId };
      writeFileSync(file, JSON.stringify(carried));
    }
    await call(adminToken, "gmHeal", {
      zone: carried.zone,
      userId: carried.sid,
      hp: 7,
    });
    await waitForHp(carried.zone, carried.sid, hp, 10_000);
  },
  60_000,
);

test.skipIf(!enabled || step !== "check")(
  "check: after the restart, the zone has the healed hp",
  async () => {
    const carried = JSON.parse(readFileSync(file, "utf8")) as Carried;
    await waitForHp(carried.zone, carried.sid, hp, 30_000);
  },
  60_000,
);
