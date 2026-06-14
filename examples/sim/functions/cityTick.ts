import { mutation } from "@pylonsync/functions";

/**
 * The simulation. A self-perpetuating scheduled chain (same pattern as
 * the world3d building-rebuild sweep): each run advances the economy
 * and grows buildings, then reschedules itself. Persisted jobs survive
 * restarts; ensureCity revives the chain if its heartbeat goes stale.
 *
 * Growth rules (legible on purpose):
 *   - a zone tile only develops if it's adjacent to a road
 *   - every road-served zone gets at least a level-1 building
 *   - higher levels are gated by demand: housing wants jobs, shops and
 *     factories want residents — the classic R/C/I feedback loop
 *
 * Constants mirror game/config.ts (functions can't import it).
 */
const GRID = 64;
const TICK_MS = 6000;
const MAX_LEVEL = 3;
const RES_CAPACITY = [0, 8, 28, 70];
const JOB_CAPACITY = [0, 6, 20, 52];

function bucket(demand: number): number {
  if (demand >= 50) return 3;
  if (demand >= 20) return 2;
  return 1;
}

export default mutation({
  auth: "guest",
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    await ctx.db.advisoryLock("sim.cityTick");

    const city = await ctx.db.lookup("City", "key", "main");
    if (!city) return { ticked: false, reason: "no_city" };

    const now = Date.now();
    // Chain dedupe: if the heartbeat is well in the future, another
    // chain owns the cadence — let it run and end this one.
    const due = new Date(city.nextTickAt as string).getTime();
    if (Number.isFinite(due) && due > now + 60_000) {
      return { ticked: false, reason: "duplicate_chain" };
    }

    const tiles = await ctx.db.list("Tile");
    const roadSet = new Set<string>();
    for (const t of tiles) {
      if (t.kind === "road") roadSet.add(`${t.gx},${t.gz}`);
    }
    const served = (gx: number, gz: number) =>
      roadSet.has(`${gx + 1},${gz}`) ||
      roadSet.has(`${gx - 1},${gz}`) ||
      roadSet.has(`${gx},${gz + 1}`) ||
      roadSet.has(`${gx},${gz - 1}`);

    // Current capacity from existing levels.
    let population = 0;
    let jobs = 0;
    for (const t of tiles) {
      const lvl = Math.max(0, Math.min(MAX_LEVEL, Math.round((t.level as number) ?? 0)));
      if (t.kind === "res") population += RES_CAPACITY[lvl];
      else if (t.kind === "com" || t.kind === "ind") jobs += JOB_CAPACITY[lvl];
    }

    // Demand → target development level per zone type.
    const resTarget = Math.max(1, bucket(jobs * 1.2 + 12 - population));
    const comTarget = bucket(population);
    const indTarget = bucket(population * 0.8);
    const targetFor = (kind: string) =>
      kind === "res" ? resTarget : kind === "com" ? comTarget : indTarget;

    // Per-cell density character: most residential lots stay low (single
    // houses), some become mid-rise, a few become towers — so the city
    // reads as a varied neighbourhood (mostly houses + the odd highrise)
    // rather than a uniform wall of max-level blocks. Commercial/
    // industrial lean denser. Deterministic from the cell.
    const cellCap = (gx: number, gz: number, kind: string): number => {
      let h = Math.abs(Math.sin(gx * 92.41 + gz * 41.17) * 24634.17);
      h -= Math.floor(h);
      if (kind === "res") return h < 0.62 ? 1 : h < 0.88 ? 2 : 3;
      return h < 0.3 ? 1 : h < 0.7 ? 2 : 3;
    };

    // Grow each road-served zone one step toward its (capped) target.
    const nowIso = new Date(now).toISOString();
    let grown = 0;
    for (const t of tiles) {
      if (t.kind === "road") continue;
      if (!served(t.gx as number, t.gz as number)) continue;
      const lvl = Math.max(0, Math.min(MAX_LEVEL, Math.round((t.level as number) ?? 0)));
      const cap = cellCap(t.gx as number, t.gz as number, t.kind as string);
      const target = Math.min(MAX_LEVEL, targetFor(t.kind as string), cap);
      if (lvl < target) {
        // unsafe: `level` is server-owned (Tile update policy is false).
        await ctx.db.unsafe.update("Tile", t.id as string, {
          level: lvl + 1,
          updatedAt: nowIso,
        });
        grown++;
        const k = t.kind as string;
        if (k === "res") population += RES_CAPACITY[lvl + 1] - RES_CAPACITY[lvl];
        else jobs += JOB_CAPACITY[lvl + 1] - JOB_CAPACITY[lvl];
      }
    }

    // Economy: tax income minus road upkeep.
    const tax = Math.round(population * 0.6 + jobs * 0.45);
    const upkeep = Math.round(roadSet.size * 0.2);
    const funds = (city.funds as number) + tax - upkeep;

    // Happiness: unemployment / housing-shortage drag.
    const balance = Math.min(population, jobs * 1.5) / Math.max(1, population);
    const happiness = Math.round(60 + 40 * (population === 0 ? 1 : balance));

    // unsafe: City is server-owned.
    await ctx.db.unsafe.update("City", city.id as string, {
      funds,
      population,
      jobs,
      happiness,
      tick: ((city.tick as number) ?? 0) + 1,
      nextTickAt: new Date(now + TICK_MS).toISOString(),
      updatedAt: nowIso,
    });

    await ctx.scheduler.runAfter(TICK_MS, "cityTick", {});
    return { ticked: true, grown, population, jobs, funds };
  },
});
