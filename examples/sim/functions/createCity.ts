import { mutation, v } from "@pylonsync/functions";

/**
 * Start a brand-new city from the lobby. Creates a City row with a fresh,
 * unique id and a starting treasury, and kicks off the city's self-
 * perpetuating simulation tick. The map starts EMPTY — no roads, no zones,
 * nothing auto-built; the player lays everything down themselves, and the sim
 * grows buildings only once roads + zones exist. Returns the new cityId so the
 * client can enter it.
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

    // unsafe: City is policy-locked to server functions. No tiles are seeded —
    // a new city is a blank canvas the player builds from scratch.
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

    await ctx.scheduler.runAfter(TICK_MS, "cityTick", { cityId });
    return { cityId, name };
  },
});
