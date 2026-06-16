import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// rejectSubmission — owner-only. Marks a submission rejected (it never becomes
// a public Listing). Kept (not deleted) so the owner has a record; a real app
// might purge rejected rows on a schedule.
export default mutation<{ submissionId: string }, { ok: boolean }>({
  auth: "user",
  args: { submissionId: v.id("Submission") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can moderate submissions.");
    }
    const sub = (await ctx.db.get("Submission", args.submissionId)) as { id: string } | null;
    if (!sub) throw ctx.error("NOT_FOUND", "Submission not found.");
    await ctx.db.unsafe.update("Submission", args.submissionId, { status: "rejected" });
    return { ok: true };
  },
});
