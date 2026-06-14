import { mutation } from "@pylonsync/functions";

/**
 * Bulldoze the entire shared city and refill the treasury. Any signed-
 * in player can reset — it's a demo sandbox. The starter town is laid
 * back down so the map is never left empty.
 */
const START_FUNDS = 8000;

function seedTiles(): Array<{ gx: number; gz: number; kind: string; level: number }> {
  // Mirror of ensureCity.seedTiles — a RANDOMISED starter neighbourhood
  // so every "new city" is a different layout.
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
  const R = 10 + Math.floor(rnd(7, 7) * 3);
  const spacing = 3 + Math.floor(rnd(3, 9) * 3);
  const density = 0.62 + rnd(5, 5) * 0.18;
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
      if (!adj) continue;
      const h = rnd(gx, gz);
      if (h > density) continue;
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

    // Reconcile the existing city into the new layout instead of bulldozing
    // it (delete-all + insert-all). The old and new neighbourhoods overlap
    // heavily on the grid, so most cells are an in-place kind/level UPDATE,
    // and only the symmetric difference is inserted/deleted. The city never
    // passes through an empty state — it morphs — and the change-set (hence
    // the sync churn every co-op client has to replay) is ~halved.
    const existing = await ctx.db.list("Tile");
    const byCell = new Map<string, (typeof existing)[number]>();
    for (const t of existing) byCell.set(`${t.gx},${t.gz}`, t);

    const nowIso = new Date().toISOString();
    const nextKeys = new Set<string>();
    let inserted = 0;
    let updated = 0;
    let deleted = 0;

    for (const t of seedTiles()) {
      const key = `${t.gx},${t.gz}`;
      nextKeys.add(key);
      const cur = byCell.get(key);
      if (cur) {
        if (cur.kind !== t.kind || cur.level !== t.level) {
          // unsafe: `level`/`kind` are server-owned (Tile update policy is false).
          await ctx.db.unsafe.update("Tile", cur.id as string, {
            kind: t.kind,
            level: t.level,
            userId: "seed",
            updatedAt: nowIso,
          });
          updated++;
        }
      } else {
        // unsafe: server-owned seed inserts (no charge, userId "seed").
        await ctx.db.unsafe.insert("Tile", {
          gx: t.gx,
          gz: t.gz,
          kind: t.kind,
          level: t.level,
          userId: "seed",
          updatedAt: nowIso,
        });
        inserted++;
      }
    }

    // Drop cells the new layout no longer uses.
    for (const t of existing) {
      if (!nextKeys.has(`${t.gx},${t.gz}`)) {
        await ctx.db.delete("Tile", t.id as string);
        deleted++;
      }
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
    return { reset: true, inserted, updated, deleted, removed: deleted };
  },
});
