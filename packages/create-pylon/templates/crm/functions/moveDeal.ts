import { mutation, v } from "@pylonsync/functions";
import { isValidStage, stageById } from "../lib/pipeline";

/**
 * Move a deal to another stage, and log the move on its timeline.
 *
 * A mutation rather than a plain `db.update` from the board, because two things
 * have to happen together: the stage changes and the history records who moved
 * it and when. A forecast you can't audit is a forecast nobody trusts.
 *
 * The stage is validated server-side — the board only ever sends known stages,
 * but the endpoint is reachable directly and an unknown stage would strand the
 * deal in a column that doesn't render.
 */
export default mutation<
  { dealId: string; stage: string },
  { ok: true }
>({
  auth: "user",
  args: { dealId: v.id("Deal"), stage: v.string() },
  async handler(ctx, args) {
    if (!isValidStage(args.stage)) {
      throw ctx.error("INVALID_ARGS", `Unknown stage "${args.stage}".`);
    }

    const deal = await ctx.db.get("Deal", args.dealId);
    if (!deal) throw ctx.error("NOT_FOUND", "Deal not found.");

    const from = String(deal.stage ?? "");
    // Re-dropping a card in its own column reaches here on a fast double-drag.
    // Nothing changed, so don't write a history entry claiming it did.
    if (from === args.stage) return { ok: true } as const;

    const now = new Date().toISOString();
    await ctx.db.update("Deal", args.dealId, {
      stage: args.stage,
      updatedAt: now,
    });

    await ctx.db.insert("Activity", {
      kind: "note",
      body: `Moved from ${stageById(from)?.label ?? from} to ${
        stageById(args.stage)?.label ?? args.stage
      }`,
      dealId: args.dealId,
      companyId: deal.companyId ?? null,
      ownerId: ctx.auth.userId,
      createdAt: now,
    });

    return { ok: true } as const;
  },
});
