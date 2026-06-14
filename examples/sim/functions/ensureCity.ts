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
  // A starter neighbourhood: a grid of streets (every 4 cells) with
  // zones lining them and block interiors left open for trees. Mostly
  // residential, some commercial/industrial; pre-grown to L1-2 so the
  // city looks alive on first load. Deterministic from the cell coords.
  const cells = new Map<string, { gx: number; gz: number; kind: string; level: number }>();
  const set = (gx: number, gz: number, kind: string, level: number) => {
    const k = gx + "," + gz;
    if (!cells.has(k)) cells.set(k, { gx, gz, kind, level });
  };
  const C = 64;
  const R = 10;
  const isRoad = (gx: number, gz: number) => (gx - C) % 4 === 0 || (gz - C) % 4 === 0;
  for (let gx = C - R; gx <= C + R; gx++) {
    for (let gz = C - R; gz <= C + R; gz++) {
      if (isRoad(gx, gz)) {
        set(gx, gz, "road", 0);
        continue;
      }
      const adj =
        isRoad(gx + 1, gz) || isRoad(gx - 1, gz) || isRoad(gx, gz + 1) || isRoad(gx, gz - 1);
      if (!adj) continue; // interior → trees
      let h = Math.abs(Math.sin(gx * 12.9898 + gz * 78.233) * 43758.5453);
      h -= Math.floor(h);
      if (h < 0.3) continue; // some empty lots
      const kind = h < 0.76 ? "res" : h < 0.9 ? "com" : "ind";
      set(gx, gz, kind, h < 0.55 ? 1 : 2);
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
