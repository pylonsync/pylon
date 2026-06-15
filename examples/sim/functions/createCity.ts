import { mutation, v } from "@pylonsync/functions";
import { seedTiles } from "./lib/seed";

/**
 * Start a brand-new city from the lobby. Creates a City row with a fresh,
 * unique id, lays down a randomised starter neighbourhood scoped to that city,
 * and kicks off the city's self-perpetuating simulation tick. Returns the new
 * cityId so the client can enter it.
 */
const TICK_MS = 6000;
const START_FUNDS = 8000;

function slugify(name: string): string {
  return (
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 24) || "city"
  );
}

export default mutation({
  auth: "guest",
  args: { name: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const name = String(args.name ?? "").trim().slice(0, 40) || "New City";
    // A readable slug plus a short random suffix keeps the id unique even if
    // two players name their cities the same thing.
    const cityId = `${slugify(name)}-${Math.random().toString(36).slice(2, 8)}`;

    const now = Date.now();
    const nowIso = new Date(now).toISOString();
    const nextTickAt = new Date(now + TICK_MS).toISOString();

    // unsafe: City is policy-locked to server functions.
    await ctx.db.unsafe.insert("City", {
      key: cityId,
      name,
      funds: START_FUNDS,
      population: 0,
      jobs: 0,
      happiness: 100,
      tick: 0,
      nextTickAt,
      updatedAt: nowIso,
    });

    // Seed the starter neighbourhood, scoped to this city (server-owned).
    for (const t of seedTiles()) {
      await ctx.db.unsafe.insert("Tile", {
        cityId,
        gx: t.gx,
        gz: t.gz,
        kind: t.kind,
        level: t.level,
        userId: "seed",
        updatedAt: nowIso,
      });
    }

    await ctx.scheduler.runAfter(TICK_MS, "cityTick", { cityId });
    return { cityId, name };
  },
});
