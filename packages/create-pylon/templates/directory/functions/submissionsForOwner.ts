import { query } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { SubmissionRow, OwnerSubmissionsResult } from "../lib/directory";

// submissionsForOwner — the owner's moderation queue, INCLUDING the submitter's
// name + email. The one function allowed to return that PII, gated to the
// configured owner (PYLON_OWNER_EMAIL via ctx.env). A query has no `ctx.error`,
// so a non-owner gets `{ authorized: false }` and NO data.
//
// The dashboard calls it with `callFn` and re-fetches whenever the public
// Listing set changes (approving a submission creates a Listing) — so the queue
// refreshes without a reload, while submitter contact details never sync.
export default query({
  auth: "user",
  async handler(ctx): Promise<OwnerSubmissionsResult> {
    const me = await ctx.db.get("User", ctx.auth.userId);
    const email = (me?.email as string | undefined) ?? null;
    if (!emailMatchesOwner(email, ctx.env.PYLON_OWNER_EMAIL)) {
      return { authorized: false };
    }

    const rows = (await ctx.db.unsafe.list("Submission")) as unknown as SubmissionRow[];
    const submissions = rows
      .map((r) => ({ ...r }))
      .sort((a, b) => (a.createdAt < b.createdAt ? 1 : a.createdAt > b.createdAt ? -1 : 0));

    return { authorized: true, submissions };
  },
});
