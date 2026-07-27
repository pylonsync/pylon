import { mutation, v } from "@pylonsync/functions";
import { isValidPriority } from "../lib/tickets";

/**
 * Open a ticket on a customer's behalf — the "log a phone call" path.
 *
 * Writes the ticket and its first message together so the thread is never
 * empty, and marks that first message as coming FROM the customer: it's their
 * report, typed in by an agent, and the SLA clock should start unanswered.
 */
export default mutation<
  {
    subject: string;
    body: string;
    customerId?: string;
    priority?: string;
  },
  { ok: true; id: string }
>({
  auth: "user",
  args: {
    subject: v.string(),
    body: v.string(),
    customerId: v.optional(v.id("Customer")),
    priority: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const subject = args.subject.trim();
    if (!subject) throw ctx.error("INVALID_ARGS", "A ticket needs a subject.");

    const priority = args.priority ?? "normal";
    if (!isValidPriority(priority)) {
      throw ctx.error("INVALID_ARGS", `Unknown priority "${priority}".`);
    }

    const now = new Date().toISOString();
    const ticketId = await ctx.db.insert("Ticket", {
      subject,
      status: "open",
      priority,
      customerId: args.customerId ?? null,
      assigneeId: null,
      firstRespondedAt: null,
      createdAt: now,
      updatedAt: now,
    });

    const body = args.body.trim();
    if (body) {
      await ctx.db.insert("Message", {
        ticketId,
        body,
        fromCustomer: true,
        internal: false,
        authorId: null,
        createdAt: now,
      });
    }

    return { ok: true, id: ticketId as string } as const;
  },
});
