import { mutation, v } from "@pylonsync/functions";

// Internal: settle a Generation — done (with a resultUrl) or failed (with an
// error). The update syncs to the user's open tabs, flipping the "generating…"
// card to the finished result live. Called only by the generate action.
export default mutation<
  {
    id: string;
    status: "done" | "failed";
    resultUrl?: string | null;
    error?: string | null;
    demo?: boolean;
  },
  { ok: boolean }
>({
  internal: true,
  args: {
    id: v.id("Generation"),
    status: v.string(),
    resultUrl: v.optional(v.string()),
    error: v.optional(v.string()),
    demo: v.optional(v.boolean()),
  },
  async handler(ctx, args) {
    await ctx.db.unsafe.update("Generation", args.id, {
      status: args.status,
      resultUrl: args.resultUrl ?? null,
      error: args.error ?? null,
      demo: args.demo ?? false,
    });
    return { ok: true };
  },
});
