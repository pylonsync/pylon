import { mutation, v } from "@pylonsync/functions";
import { isValidPriority, isValidStatus } from "../lib/tickets";

/**
 * Change a ticket's status, priority, or assignee.
 *
 * One mutation for all three because they share the same validation and the
 * same `updatedAt` bump, and because a queue that accepts an unknown status
 * strands the ticket in a filter nothing renders. Every value is checked
 * server-side — the UI only ever offers known ones, but the endpoint is
 * reachable directly.
 *
 * `assigneeId: null` explicitly unassigns; omitting it leaves the owner alone.
 */
export default mutation<
  {
    ticketId: string;
    status?: string;
    priority?: string;
    assigneeId?: string | null;
  },
  { ok: true }
>({
  auth: "user",
  args: {
    ticketId: v.id("Ticket"),
    status: v.optional(v.string()),
    priority: v.optional(v.string()),
    assigneeId: v.optional(v.id("User")),
  },
  async handler(ctx, args) {
    const ticket = await ctx.db.get("Ticket", args.ticketId);
    if (!ticket) throw ctx.error("NOT_FOUND", "Ticket not found.");

    const patch: Record<string, unknown> = { updatedAt: new Date().toISOString() };

    if (args.status !== undefined) {
      if (!isValidStatus(args.status)) {
        throw ctx.error("INVALID_ARGS", `Unknown status "${args.status}".`);
      }
      patch.status = args.status;
    }

    if (args.priority !== undefined) {
      if (!isValidPriority(args.priority)) {
        throw ctx.error("INVALID_ARGS", `Unknown priority "${args.priority}".`);
      }
      patch.priority = args.priority;
    }

    if (args.assigneeId !== undefined) {
      patch.assigneeId = args.assigneeId || null;
    }

    await ctx.db.update("Ticket", args.ticketId, patch);
    return { ok: true } as const;
  },
});
