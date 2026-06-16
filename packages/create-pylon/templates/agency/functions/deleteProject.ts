import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// deleteProject — owner-only. Removes a case study. Any invoice that referenced
// it keeps its denormalized `projectTitle` (so the bill still reads sensibly),
// it just no longer links anywhere — we don't cascade-delete money records.
export default mutation<{ id: string }, { ok: boolean }>({
  auth: "user",
  args: { id: v.id("Project") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage projects.");
    }
    const project = await ctx.db.get("Project", args.id);
    if (!project) return { ok: true };
    await ctx.db.unsafe.delete("Project", args.id);
    return { ok: true };
  },
});
