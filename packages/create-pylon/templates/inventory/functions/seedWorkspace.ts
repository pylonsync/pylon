import { mutation } from "@pylonsync/functions";
import { SEED_TEAM, shapeSeed } from "../lib/seed";
import { ensureDemoUser } from "../lib/demo-team";

/**
 * Fill a brand-new stock workspace once.
 *
 * Seeds a plausible SEQUENCE of receipts and sales rather than a starting
 * quantity, because on-hand here is the sum of the ledger; a seed that wrote
 * levels directly would contradict the app's own model.
 *
 * Returns immediately if any product exists, so it is safe on every load. An
 * advisory lock stops two first loads from seeding twice. Movements are
 * recorded by the person who signed in and four demo teammates (User rows
 * with no password, see SEED_TEAM). Delete this function, lib/seed.ts, and the
 * `seedWorkspace` call in app/workspace.tsx once you stock real products, then
 * delete the demo users.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  args: {},
  async handler(ctx) {
    if ((await ctx.db.query("Product", { $limit: 1 })).length > 0) return { seeded: false };
    await ctx.db.advisoryLock("inventory_seed_workspace");
    if ((await ctx.db.query("Product", { $limit: 1 })).length > 0) return { seeded: false };

    const seed = shapeSeed();
    const me = ctx.auth.userId;

    // Position 0 is the person who signed in; SEED_TEAM follows.
    const team: Array<string | null> = [me];
    for (const person of SEED_TEAM) team.push(await ensureDemoUser(ctx.db, person));

    const productIds = new Map<string, string>();
    for (const product of seed.products) {
      const id = await ctx.db.insert("Product", product.row);
      productIds.set(product.key, id as string);
    }

    for (const movement of seed.movements) {
      await ctx.db.insert("Movement", {
        ...movement.row,
        productId: productIds.get(movement.product) ?? null,
        actorId: team[movement.actor] ?? me,
      });
    }

    return { seeded: true };
  },
});
