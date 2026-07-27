import { mutation } from "@pylonsync/functions";
import { shapeSeed } from "../lib/seed";

/**
 * Fill a brand-new helpdesk with a realistic queue, once.
 *
 * An empty inbox demonstrates nothing about triage, SLA, or the thread view.
 * The client calls this after sign-in; it returns immediately if any ticket
 * already exists, so it is safe on every load and can never duplicate the
 * fixtures or touch real data.
 *
 * Delete this function and lib/seed.ts once real tickets arrive.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  args: {},
  async handler(ctx) {
    const existing = await ctx.db.query("Ticket", { $limit: 1 });
    if (existing.length > 0) return { seeded: false };

    const seed = shapeSeed();
    const me = ctx.auth.userId;

    const customerIds = new Map<string, string>();
    for (const customer of seed.customers) {
      const id = await ctx.db.insert("Customer", customer.row);
      customerIds.set(customer.key, id as string);
    }

    const ticketIds = new Map<string, string>();
    for (const ticket of seed.tickets) {
      const id = await ctx.db.insert("Ticket", {
        ...ticket.row,
        customerId: customerIds.get(ticket.customer) ?? null,
        // Answered tickets are assigned to whoever is seeding; the unanswered
        // ones stay unassigned so the queue has something to pick up.
        assigneeId: ticket.row.firstRespondedAt ? me : null,
      });
      ticketIds.set(ticket.key, id as string);
    }

    for (const message of seed.messages) {
      await ctx.db.insert("Message", {
        ...message.row,
        ticketId: ticketIds.get(message.ticket) ?? null,
        authorId: message.row.fromCustomer ? null : me,
      });
    }

    return { seeded: true };
  },
});
