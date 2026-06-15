import { mutation, v } from "@pylonsync/functions";
import { seedTiles } from "./lib/seed";

/**
 * Re-roll one city's layout and refill its treasury. Any signed-in player can
 * reset — it's a demo sandbox. The starter town is laid back down so the map
 * is never left empty.
 */
const START_FUNDS = 8000;

export default mutation({
  auth: "guest",
  args: { cityId: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const cityId = String(args.cityId ?? "");
    if (!cityId) throw ctx.error("INVALID_ARGS", "cityId required");

    // Reconcile this city's existing tiles into the new layout instead of
    // bulldozing it (delete-all + insert-all). The old and new neighbourhoods
    // overlap heavily on the grid, so most cells are an in-place kind/level
    // UPDATE, and only the symmetric difference is inserted/deleted. The city
    // never passes through an empty state — it morphs — and the change-set
    // (hence the sync churn every co-op client replays) is ~halved.
    const existing = await ctx.db.query("Tile", { cityId });
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
          cityId,
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

    // Drop cells the new layout no longer uses (within this city only).
    for (const t of existing) {
      if (!nextKeys.has(`${t.gx},${t.gz}`)) {
        await ctx.db.delete("Tile", t.id as string);
        deleted++;
      }
    }

    const city = await ctx.db.lookup("City", "key", cityId);
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
