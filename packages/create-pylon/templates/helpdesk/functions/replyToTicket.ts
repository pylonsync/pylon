import { mutation, v } from "@pylonsync/functions";

/**
 * Post an agent message on a ticket.
 *
 * A mutation rather than a bare insert because a reply changes the ticket too:
 * the first public reply stamps `firstRespondedAt`, which is what the SLA state
 * is computed from. Doing that client-side would let a slow or offline tab
 * decide when the clock stopped.
 *
 * An INTERNAL note deliberately does NOT stop the clock — the customer is still
 * waiting, and a team that could clear its SLA by talking to itself would have
 * an SLA worth nothing.
 */
export default mutation<
  { ticketId: string; body: string; internal?: boolean },
  { ok: true; id: string }
>({
  auth: "user",
  args: {
    ticketId: v.id("Ticket"),
    body: v.string(),
    internal: v.optional(v.boolean()),
  },
  async handler(ctx, args) {
    const body = args.body.trim();
    if (!body) throw ctx.error("INVALID_ARGS", "A reply needs a body.");

    const ticket = await ctx.db.get("Ticket", args.ticketId);
    if (!ticket) throw ctx.error("NOT_FOUND", "Ticket not found.");

    const internal = args.internal ?? false;
    const now = new Date().toISOString();

    const id = await ctx.db.insert("Message", {
      ticketId: args.ticketId,
      body,
      fromCustomer: false,
      internal,
      authorId: ctx.auth.userId,
      createdAt: now,
    });

    const patch: Record<string, unknown> = { updatedAt: now };
    if (!internal && !ticket.firstRespondedAt) {
      patch.firstRespondedAt = now;
    }
    // Replying to something nobody owns takes ownership — the common case, and
    // it stops two agents answering the same ticket.
    if (!ticket.assigneeId) patch.assigneeId = ctx.auth.userId;
    await ctx.db.update("Ticket", args.ticketId, patch);

    return { ok: true, id: id as string } as const;
  },
});
