import { mutation } from "@pylonsync/functions";

/**
 * Bulldoze the entire shared city and refill the treasury. Any signed-
 * in player can reset — it's a demo sandbox. The starter town is laid
 * back down so the map is never left empty.
 */
const START_FUNDS = 8000;

function seedTiles(): Array<{ gx: number; gz: number; kind: string; level: number }> {
  // Mirror of ensureCity.seedTiles — a starter neighbourhood grid.
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
      if (!adj) continue;
      let h = Math.abs(Math.sin(gx * 12.9898 + gz * 78.233) * 43758.5453);
      h -= Math.floor(h);
      if (h < 0.3) continue;
      const kind = h < 0.76 ? "res" : h < 0.9 ? "com" : "ind";
      set(gx, gz, kind, h < 0.72 ? 1 : 2);
    }
  }
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
        level: t.level,
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
