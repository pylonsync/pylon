import { mutation } from "@pylonsync/functions";

/**
 * Bulldoze the entire shared city and refill the treasury. Any signed-
 * in player can reset — it's a demo sandbox. The starter town is laid
 * back down so the map is never left empty.
 */
const START_FUNDS = 8000;

function seedTiles(): Array<{ gx: number; gz: number; kind: string }> {
  const cells = new Map<string, { gx: number; gz: number; kind: string }>();
  const set = (gx: number, gz: number, kind: string) => {
    const k = gx + "," + gz;
    if (!cells.has(k)) cells.set(k, { gx, gz, kind });
  };
  const C = 64;
  for (let i = -6; i <= 6; i++) {
    set(C + i, C, "road");
    set(C, C + i, "road");
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

    const tiles = await ctx.db.list("Tile");
    for (const t of tiles) await ctx.db.delete("Tile", t.id as string);

    const nowIso = new Date().toISOString();
    for (const t of seedTiles()) {
      // unsafe: server-owned seed inserts (no charge, userId "seed").
      await ctx.db.unsafe.insert("Tile", {
        gx: t.gx,
        gz: t.gz,
        kind: t.kind,
        level: 0,
        userId: "seed",
        updatedAt: nowIso,
      });
    }

    const city = await ctx.db.lookup("City", "key", "main");
    if (city) {
      await ctx.db.unsafe.update("City", city.id as string, {
        funds: START_FUNDS,
        population: 0,
        jobs: 0,
        happiness: 100,
        updatedAt: nowIso,
      });
    }
    return { reset: true, removed: tiles.length };
  },
});
