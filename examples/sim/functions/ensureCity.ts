import { mutation } from "@pylonsync/functions";

/**
 * Idempotent bootstrap for the one shared city. The client calls this
 * on load; the first caller creates the City row (starting treasury)
 * and starts the self-perpetuating simulation tick. Later callers just
 * make sure the tick chain is alive (revives it after a restart /
 * wiped job store) under an advisory lock so concurrent joins can't
 * double-start it.
 */
const TICK_MS = 6000;
const START_FUNDS = 8000;

/**
 * A small starter neighbourhood so the map isn't an empty shell on
 * first load — a cross of main streets with zones beside them (all
 * road-served, so the sim grows them immediately).
 */
function seedTiles(): Array<{ gx: number; gz: number; kind: string; level: number }> {
  // A starter neighbourhood: a grid of streets with zones lining them and
  // block interiors left open for trees. RANDOMISED per call (street
  // spacing, district size, mix, density) so every "new city" is a
  // genuinely different layout. The generated tiles are stored + synced,
  // so all co-op clients still see the one shared city.
  const seed = ((Math.random() * 0xffffffff) >>> 0) || 1;
  const rnd = (a: number, b: number): number => {
    let h = (Math.imul(a | 0, 73856093) ^ Math.imul(b | 0, 19349663) ^ seed) >>> 0;
    h = Math.imul(h ^ (h >>> 16), 2246822507) >>> 0;
    h = Math.imul(h ^ (h >>> 13), 3266489909) >>> 0;
    return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
  };
  const cells = new Map<string, { gx: number; gz: number; kind: string; level: number }>();
  const set = (gx: number, gz: number, kind: string, level: number) => {
    const k = gx + "," + gz;
    if (!cells.has(k)) cells.set(k, { gx, gz, kind, level });
  };
  const C = 64;
  const R = 10 + Math.floor(rnd(7, 7) * 3); // 10..12
  const spacing = 3 + Math.floor(rnd(3, 9) * 3); // streets every 3..5 cells
  const density = 0.82 + rnd(5, 5) * 0.13; // fraction of road-served lots built
  const isRoad = (gx: number, gz: number) =>
    (gx - C) % spacing === 0 || (gz - C) % spacing === 0;
  for (let gx = C - R; gx <= C + R; gx++) {
    for (let gz = C - R; gz <= C + R; gz++) {
      if (isRoad(gx, gz)) {
        set(gx, gz, "road", 0);
        continue;
      }
      const adj =
        isRoad(gx + 1, gz) || isRoad(gx - 1, gz) || isRoad(gx, gz + 1) || isRoad(gx, gz - 1);
      if (!adj) continue; // interior → trees
      const h = rnd(gx, gz);
      if (h > density) continue; // empty lot → trees
      const k = rnd(gx * 7 + 1, gz * 13 + 1);
      const kind = k < 0.74 ? "res" : k < 0.9 ? "com" : "ind";
      set(gx, gz, kind, rnd(gx + 5, gz + 5) < 0.72 ? 1 : 2);
    }
  }
  return [...cells.values()];
}

export default mutation({
  auth: "guest",
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    await ctx.db.advisoryLock("sim.city");
    const now = Date.now();
    const city = await ctx.db.lookup("City", "key", "main");

    if (!city) {
      const nextTickAt = new Date(now + TICK_MS).toISOString();
      const nowIso = new Date(now).toISOString();
      // unsafe: City is policy-locked to server functions.
      await ctx.db.unsafe.insert("City", {
        key: "main",
        funds: START_FUNDS,
        population: 0,
        jobs: 0,
        happiness: 100,
        tick: 0,
        nextTickAt,
        updatedAt: nowIso,
      });
      // Seed the starter neighbourhood (server-owned inserts, no charge).
      for (const t of seedTiles()) {
        await ctx.db.unsafe.insert("Tile", {
          gx: t.gx,
          gz: t.gz,
          kind: t.kind,
          level: t.level,
          userId: "seed",
          updatedAt: nowIso,
        });
      }
      await ctx.scheduler.runAfter(TICK_MS, "cityTick", {});
      return { created: true };
    }

    // Revive the tick chain if its heartbeat is missing or stale.
    const due = new Date(city.nextTickAt as string).getTime();
    if (!Number.isFinite(due) || due < now - 60_000) {
      const nextTickAt = new Date(now + TICK_MS).toISOString();
      await ctx.db.unsafe.update("City", city.id as string, { nextTickAt });
      await ctx.scheduler.runAfter(TICK_MS, "cityTick", {});
    }
    return { created: false };
  },
});
