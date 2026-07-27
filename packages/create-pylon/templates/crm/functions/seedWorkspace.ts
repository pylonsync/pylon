import { mutation } from "@pylonsync/functions";
import { shapeSeed } from "../lib/seed";

/**
 * Fill a brand-new workspace with a realistic pipeline, once.
 *
 * An empty CRM demonstrates nothing — no board, no forecast, no sense of what
 * the app is for. The client calls this after sign-in; it returns immediately
 * if any company already exists, so it is safe to call on every load and can
 * never duplicate the fixtures or touch real data.
 *
 * Delete this function and lib/seed.ts once the workspace has real customers.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  args: {},
  async handler(ctx) {
    // The guard is "has anything at all", not "has the seed", so a workspace
    // whose demo rows were deleted on purpose stays deleted.
    const existing = await ctx.db.query("Company", { $limit: 1 });
    if (existing.length > 0) return { seeded: false };

    const seed = shapeSeed();
    const owner = ctx.auth.userId;

    const companyIds = new Map<string, string>();
    for (const company of seed.companies) {
      const id = await ctx.db.insert("Company", { ...company.row, ownerId: owner });
      companyIds.set(company.key, id as string);
    }

    const contactIds = new Map<string, string>();
    for (const contact of seed.contacts) {
      const id = await ctx.db.insert("Contact", {
        ...contact.row,
        companyId: companyIds.get(contact.company) ?? null,
        ownerId: owner,
      });
      contactIds.set(String(contact.row.name), id as string);
    }

    const dealIds = new Map<string, string>();
    for (const deal of seed.deals) {
      const id = await ctx.db.insert("Deal", {
        ...deal.row,
        companyId: companyIds.get(deal.company) ?? null,
        contactId: contactIds.get(deal.contact) ?? null,
        ownerId: owner,
      });
      dealIds.set(deal.title, id as string);
    }

    for (const activity of seed.activities) {
      await ctx.db.insert("Activity", {
        ...activity.row,
        dealId: dealIds.get(activity.deal) ?? null,
        ownerId: owner,
      });
    }

    return { seeded: true };
  },
});
