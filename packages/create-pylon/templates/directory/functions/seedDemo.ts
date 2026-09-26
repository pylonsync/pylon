import { mutation } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import { DEMO_SUBMISSIONS, demoDataEnabled } from "../lib/demo";

// seedDemo — owner-only, development-only. Fills an empty review queue with the
// fictional submissions in lib/demo.ts, so the curator dashboard has entries.
//
// It writes nothing unless demo data is on (lib/demo.ts: PYLON_DEMO_DATA, else
// a `pylon dev` process), so `pylon start`, Docker, and Pylon Cloud deploys do
// not get demo rows. It also writes nothing once any Submission exists.
// Submissions carry submitter emails (PII), so only the signed-in owner can run
// it, like the rest of the curator surface.
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  async handler(ctx) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can seed the review queue.");
    }
    if (!demoDataEnabled(ctx.env)) return { seeded: false };

    await ctx.db.advisoryLock("directory_seed_demo");
    const existing = await ctx.db.unsafe.list("Submission");
    if (existing.length > 0) return { seeded: false };

    const nowMs = Date.now();
    for (const s of DEMO_SUBMISSIONS) {
      await ctx.db.unsafe.insert("Submission", {
        submitterName: s.submitterName,
        submitterEmail: s.submitterEmail,
        name: s.name,
        tagline: s.tagline,
        url: s.url,
        category: s.category,
        tags: s.tags,
        description: s.description || null,
        status: s.status,
        createdAt: new Date(nowMs - s.hoursAgo * 3_600_000).toISOString(),
      });
    }
    return { seeded: true };
  },
});
