import { mutation, v } from "@pylonsync/functions";
import { ACTIVITY_KINDS } from "../lib/pipeline";

/** Log a note, call, email, or meeting against a deal and/or contact. */
export default mutation<
  {
    kind: string;
    body: string;
    dealId?: string;
    contactId?: string;
    companyId?: string;
  },
  { ok: true; id: string }
>({
  auth: "user",
  args: {
    kind: v.string(),
    body: v.string(),
    dealId: v.optional(v.id("Deal")),
    contactId: v.optional(v.id("Contact")),
    companyId: v.optional(v.id("Company")),
  },
  async handler(ctx, args) {
    const body = args.body.trim();
    if (!body) throw ctx.error("INVALID_ARGS", "An activity needs a body.");
    if (!(ACTIVITY_KINDS as readonly string[]).includes(args.kind)) {
      throw ctx.error("INVALID_ARGS", `Unknown activity kind "${args.kind}".`);
    }

    const id = await ctx.db.insert("Activity", {
      kind: args.kind,
      body,
      dealId: args.dealId ?? null,
      contactId: args.contactId ?? null,
      companyId: args.companyId ?? null,
      ownerId: ctx.auth.userId,
      createdAt: new Date().toISOString(),
    });

    // Touching the deal keeps "last activity" honest without a second query.
    if (args.dealId) {
      await ctx.db.update("Deal", args.dealId, {
        updatedAt: new Date().toISOString(),
      });
    }

    return { ok: true, id: id as string } as const;
  },
});
