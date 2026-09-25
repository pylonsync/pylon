// End-to-end: a message from a shard on one machine reaches a shard on
// another (examples/shard-arena, two `pylon start` processes on one
// Postgres, no cluster bus: the machines call each other).
//
// Runs only when PYLON_SHARD_MESSAGES_E2E is "<host:port of A>,<host:port of
// B>" and PYLON_SHARD_ADMIN_TOKEN is the machines' admin token
// (tools/smoke-shard-cluster.sh sets both).

import { expect, test } from "bun:test";

import { connectShard } from "./connection";

const [hostA, hostB] = (process.env.PYLON_SHARD_MESSAGES_E2E ?? "").split(",");
const adminToken = process.env.PYLON_SHARD_ADMIN_TOKEN ?? "";

interface Zone {
  zone: string;
  heard: string[];
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
  if (res.status !== 200) throw new Error(`${fn} on ${host}: ${res.status} ${text}`);
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

test.skipIf(!hostA || !hostB || !adminToken)(
  "a message from a shard on one machine reaches a shard on another",
  async () => {
    const run = Math.random().toString(36).slice(2, 8);
    const [west, east] = [`msg-west-${run}`, `msg-east-${run}`];
    type Joined = { subscriberId: string; ticket: string; machine?: string };
    const shouter = await guest(hostA);
    const watcher = await guest(hostB);
    const w = await call<Joined>(hostA, shouter, "joinZone", { zone: west, machine: "a" });
    expect(w.machine).toBe("a");
    const e = await call<Joined>(hostB, watcher, "joinZone", {
      zone: east,
      machine: "b",
      params: { group: `realm-${run}` },
    });
    expect(e.machine).toBe("b");

    let heard: string[] = [];
    const eastClient = connectShard<Zone, unknown>(east, {
      subscriberId: e.subscriberId,
      baseUrl: hostB,
      ticket: async (shard) => (await call<Joined>(hostB, watcher, "joinZone", { zone: shard })).ticket,
    });
    eastClient.onSnapshot((s) => (heard = s.heard));
    await waitFor("east's first snapshot", () => (eastClient.tick >= 0 ? true : undefined), 10_000);

    let westTick = -1;
    const westClient = connectShard<Zone, unknown>(west, {
      subscriberId: w.subscriberId,
      baseUrl: hostA,
      ticket: async (shard) => (await call<Joined>(hostA, shouter, "joinZone", { zone: shard })).ticket,
    });
    westClient.onSnapshot((_, tick) => (westTick = tick));
    await waitFor("west's first snapshot", () => (westTick >= 0 ? true : undefined), 10_000);
    westClient.send("join");
    // To all zones, and to east's group.
    westClient.send({ shout: { text: "the relic is taken" } });
    westClient.send({ shout: { text: "to the realm", to: `group:realm-${run}` } });
    const from = `${west}> ${w.subscriberId}`;
    await waitFor(
      "east hears both",
      () =>
        heard.includes(`${from}: the relic is taken`) && heard.includes(`${from}: to the realm`)
          ? true
          : undefined,
      10_000,
    );

    // A server function on A announces to all: east, on B, hears it. A
    // player (here a guest) may not announce.
    const refused = await call(hostA, shouter, "announce", { text: "fake" }).catch((e) => e);
    expect(String(refused)).toMatch(/AUTH_REQUIRED|FORBIDDEN/);
    await call(hostA, adminToken, "announce", { text: "keep under attack" });
    await waitFor(
      "east hears the announcement",
      () => (heard.includes(`> "keep under attack"`) ? true : undefined),
      10_000,
    );
    westClient.close();
    eastClient.close();
  },
  60_000,
);
