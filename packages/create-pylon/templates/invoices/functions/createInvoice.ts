import { mutation, v } from "@pylonsync/functions";
import { nextNumber } from "../lib/billing";

/**
 * Open a draft invoice with the next number in the series.
 *
 * The number is allocated SERVER-side from the existing rows rather than by a
 * stored counter or by the client: a counter in a synced replica is a race, and
 * two drafts sharing a number is the kind of thing an accountant finds later.
 * `number` is unique in the schema, so a genuine collision fails loudly instead
 * of producing duplicate paperwork.
 */
export default mutation<
  { clientId?: string; taxRateBps?: number; dueInDays?: number },
  { ok: true; id: string; number: string }
>({
  auth: "user",
  args: {
    clientId: v.optional(v.id("Client")),
    taxRateBps: v.optional(v.number()),
    dueInDays: v.optional(v.number()),
  },
  async handler(ctx, args) {
    const rows = (await ctx.db.query("Invoice", {})) as Array<{ number?: string }>;
    const now = new Date();
    const number = nextNumber(
      rows.map((row) => String(row.number ?? "")),
      now.getUTCFullYear(),
    );

    const dueInDays = Number.isFinite(args.dueInDays) ? Number(args.dueInDays) : 30;
    const due = new Date(now.getTime() + dueInDays * 86_400_000);

    const id = await ctx.db.insert("Invoice", {
      number,
      clientId: args.clientId ?? null,
      status: "draft",
      // Basis points, integer — a percentage typed as 8.75 becomes 875 upstream.
      taxRateBps: Math.max(0, Math.round(Number(args.taxRateBps) || 0)),
      issueDate: now.toISOString(),
      dueDate: due.toISOString(),
      ownerId: ctx.auth.userId,
      createdAt: now.toISOString(),
      updatedAt: now.toISOString(),
    });

    return { ok: true, id: id as string, number } as const;
  },
});
