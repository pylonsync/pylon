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
function seedTiles(): Array<{ gx: number; gz: number; kind: string }> {
  // Keyed by cell so the two crossing roads (which share the centre)
  // and any zone overlap can't produce a duplicate (gx,gz) insert.
  const cells = new Map<string, { gx: number; gz: number; kind: string }>();
  const set = (gx: number, gz: number, kind: string) => {
    const k = gx + "," + gz;
    if (!cells.has(k)) cells.set(k, { gx, gz, kind });
  };
  const C = 32;
  for (let i = -6; i <= 6; i++) {
    set(C + i, C, "road"); // main street (E-W)
    set(C, C + i, "road"); // avenue (N-S)
  }
  for (let i = -5; i <= 5; i++) {
    if (i === 0) continue;
    set(C + i, C + 1, "res");
    set(C + i, C - 1, "res");
  }
  set(C + 1, C + 3, "com");
  set(C - 1, C + 3, "com");
  set(C + 1, C - 3, "ind");
  set(C - 1, C - 3, "ind");
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
      // Seed the starter town (server-owned inserts, no charge).
      for (const t of seedTiles()) {
        await ctx.db.unsafe.insert("Tile", {
          gx: t.gx,
          gz: t.gz,
          kind: t.kind,
          level: 0,
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
