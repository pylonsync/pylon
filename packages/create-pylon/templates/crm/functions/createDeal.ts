import { mutation, v } from "@pylonsync/functions";
import { isValidStage } from "../lib/pipeline";

/**
 * Create a deal, stamped with the signed-in user as owner.
 *
 * `ownerId` is set here rather than sent by the client: it's `.readonly()` in
 * the schema, so a client can supply it on insert but never edit it afterwards,
 * and taking it from `ctx.auth` means it can't be forged in the first place.
 */
export default mutation<
  {
    title: string;
    companyId?: string;
    contactId?: string;
    value?: number;
    stage?: string;
    closeDate?: string;
  },
  { ok: true; id: string }
>({
  auth: "user",
  args: {
    title: v.string(),
    companyId: v.optional(v.id("Company")),
    contactId: v.optional(v.id("Contact")),
    value: v.optional(v.number()),
    stage: v.optional(v.string()),
    closeDate: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const title = args.title.trim();
    if (!title) throw ctx.error("INVALID_ARGS", "A deal needs a title.");

    const stage = args.stage ?? "lead";
    if (!isValidStage(stage)) {
      throw ctx.error("INVALID_ARGS", `Unknown stage "${stage}".`);
    }

    // A negative forecast is always a typo, and it would quietly subtract from
    // every column total it lands in.
    const value = Math.max(0, Number(args.value) || 0);

    const now = new Date().toISOString();
    const id = await ctx.db.insert("Deal", {
      title,
      companyId: args.companyId ?? null,
      contactId: args.contactId ?? null,
      value,
      stage,
      closeDate: args.closeDate || null,
      ownerId: ctx.auth.userId,
      createdAt: now,
      updatedAt: now,
    });

    return { ok: true, id: id as string } as const;
  },
});
