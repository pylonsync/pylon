import { mutation } from "@pylonsync/functions";
import { SEED_TEAM, shapeSeed } from "../lib/seed";
import { ensureDemoUser } from "../lib/demo-team";

/**
 * Fill a brand-new helpdesk with a realistic queue, once.
 *
 * The client calls this after sign-in. It returns immediately if any ticket
 * already exists, so it is safe on every load and never duplicates the
 * fixtures or touches real data. An advisory lock stops two first loads from
 * seeding twice.
 *
 * Tickets are assigned across the person who signed in and four demo agents
 * (see SEED_TEAM and lib/demo-team.ts). Delete this function,
 * lib/seed.ts, and the `seedWorkspace` call in app/workspace.tsx once real
 * tickets arrive, then delete the demo users.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  args: {},
  async handler(ctx) {
    if ((await ctx.db.query("Ticket", { $limit: 1 })).length > 0) return { seeded: false };
    await ctx.db.advisoryLock("helpdesk_seed_workspace");
    if ((await ctx.db.query("Ticket", { $limit: 1 })).length > 0) return { seeded: false };

    const seed = shapeSeed();
    const me = ctx.auth.userId;

    // Position 0 is the person who signed in; SEED_TEAM follows.
    const team: Array<string | null> = [me];
    for (const person of SEED_TEAM) team.push(await ensureDemoUser(ctx.db, person));

    const customerIds = new Map<string, string>();
    for (const customer of seed.customers) {
      const id = await ctx.db.insert("Customer", customer.row);
      customerIds.set(customer.key, id as string);
    }

    const tickets = new Map<string, { id: string; assignee: string | null }>();
    for (const ticket of seed.tickets) {
      const assignee = ticket.assignee === null ? null : (team[ticket.assignee] ?? me);
      const id = await ctx.db.insert("Ticket", {
        ...ticket.row,
        customerId: customerIds.get(ticket.customer) ?? null,
        assigneeId: assignee,
      });
      tickets.set(ticket.key, { id: id as string, assignee });
    }

    // Agent replies on a ticket are written by its assignee.
    for (const message of seed.messages) {
      const ticket = tickets.get(message.ticket);
      await ctx.db.insert("Message", {
        ...message.row,
        ticketId: ticket?.id ?? null,
        authorId: message.row.fromCustomer ? null : (ticket?.assignee ?? me),
      });
    }

    return { seeded: true };
  },
});
