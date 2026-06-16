import { mutation, v } from "@pylonsync/functions";

// Internal: patch a Generation. Called by the pollGeneration background job to
// move a row through pending → processing → done/failed. Each write syncs to the
// owner's open tabs, so the gallery card updates live. Only set fields are
// touched. Not client-reachable (Generation is allowInsert/Update:"false").
export default mutation<
  {
    id: string;
    status?: string;
    resultUrl?: string | null;
    error?: string | null;
    demo?: boolean;
    predictionId?: string;
  },
  { ok: boolean }
>({
  internal: true,
  args: {
    id: v.id("Generation"),
    status: v.optional(v.string()),
    resultUrl: v.optional(v.string()),
    error: v.optional(v.string()),
    demo: v.optional(v.boolean()),
    predictionId: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const patch: Record<string, unknown> = {};
    if (args.status !== undefined) patch.status = args.status;
    if (args.resultUrl !== undefined) patch.resultUrl = args.resultUrl;
    if (args.error !== undefined) patch.error = args.error;
    if (args.demo !== undefined) patch.demo = args.demo;
    if (args.predictionId !== undefined) patch.predictionId = args.predictionId;
    await ctx.db.unsafe.update("Generation", args.id, patch);
    return { ok: true };
  },
});
