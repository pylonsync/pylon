import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// approveSubmission — owner-only. Turns a pending Submission into a PUBLIC
// Listing (copying only the PII-free fields — the submitter's name/email stay
// behind in the deny-all Submission table) and marks the submission approved.
// The new Listing syncs straight to every open browse grid.
export default mutation<{ submissionId: string }, { ok: boolean }>({
  auth: "user",
  args: { submissionId: v.id("Submission") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can moderate submissions.");
    }
    const sub = (await ctx.db.get("Submission", args.submissionId)) as
      | {
          name: string;
          tagline: string;
          url: string;
          category: string;
          tags?: string | null;
          description?: string | null;
          status: string;
        }
      | null;
    if (!sub) throw ctx.error("NOT_FOUND", "Submission not found.");
    if (sub.status === "approved") return { ok: true }; // idempotent

    // Copy ONLY the public fields — never the submitter's PII.
    await ctx.db.unsafe.insert("Listing", {
      name: sub.name,
      tagline: sub.tagline,
      url: sub.url,
      category: sub.category,
      tags: sub.tags ?? null,
      description: sub.description ?? null,
      votes: 0,
      featured: false,
      createdAt: new Date().toISOString(),
    });
    await ctx.db.unsafe.update("Submission", args.submissionId, { status: "approved" });
    return { ok: true };
  },
});
