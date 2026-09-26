import { mutation } from "@pylonsync/functions";
import { SEED_TEAM, shapeSeed } from "../lib/seed";
import { ensureDemoUser } from "../lib/demo-team";

/**
 * Fill a brand-new workspace with a realistic pipeline, once.
 *
 * The client calls this after sign-in. It returns immediately if any company
 * already exists, so it is safe to call on every load and never duplicates the
 * fixtures or touches real data. An advisory lock stops two first loads from
 * seeding twice.
 *
 * Deals are spread across the person who signed in and four demo teammates
 * (see SEED_TEAM and lib/demo-team.ts). Delete this function,
 * lib/seed.ts, and the `seedWorkspace` call in app/workspace.tsx once the
 * workspace has real customers, then delete the demo users.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  args: {},
  async handler(ctx) {
    // The guard is "has anything at all", not "has the seed", so a workspace
    // whose demo rows were deleted on purpose stays deleted.
    if ((await ctx.db.query("Company", { $limit: 1 })).length > 0) return { seeded: false };
    await ctx.db.advisoryLock("crm_seed_workspace");
    if ((await ctx.db.query("Company", { $limit: 1 })).length > 0) return { seeded: false };

    const seed = shapeSeed();
    const me = ctx.auth.userId;

    // Position 0 is the person who signed in; SEED_TEAM follows.
    const owners: Array<string | null> = [me];
    for (const person of SEED_TEAM) owners.push(await ensureDemoUser(ctx.db, person));

    const companyIds = new Map<string, string>();
    for (const [index, company] of seed.companies.entries()) {
      const id = await ctx.db.insert("Company", {
        ...company.row,
        ownerId: owners[index % owners.length],
      });
      companyIds.set(company.key, id as string);
    }

    const contactIds = new Map<string, string>();
    for (const [index, contact] of seed.contacts.entries()) {
      const id = await ctx.db.insert("Contact", {
        ...contact.row,
        companyId: companyIds.get(contact.company) ?? null,
        ownerId: owners[index % owners.length],
      });
      contactIds.set(String(contact.row.name), id as string);
    }

    const dealIds = new Map<string, { id: string; owner: string | null }>();
    for (const deal of seed.deals) {
      const owner = owners[deal.owner] ?? me;
      const id = await ctx.db.insert("Deal", {
        ...deal.row,
        companyId: companyIds.get(deal.company) ?? null,
        contactId: contactIds.get(deal.contact) ?? null,
        ownerId: owner,
      });
      dealIds.set(deal.title, { id: id as string, owner });
    }

    // Activity on a deal is logged by the deal's owner.
    for (const activity of seed.activities) {
      const deal = dealIds.get(activity.deal);
      await ctx.db.insert("Activity", {
        ...activity.row,
        dealId: deal?.id ?? null,
        ownerId: deal?.owner ?? me,
      });
    }

    return { seeded: true };
  },
});
