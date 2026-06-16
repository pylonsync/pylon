import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// setProjectFlags — owner-only. A quick toggle for the two flags the owner flips
// most: `selected` (feature it on the homepage) and `published` (show it on the
// public site at all). Separate from the full upsertProject so the dashboard can
// flip a switch without round-tripping the whole form. Pass only the flag(s) you
// want to change.
export default mutation<
  { id: string; selected?: boolean; published?: boolean },
  { ok: boolean }
>({
  auth: "user",
  args: {
    id: v.id("Project"),
    selected: v.optional(v.boolean()),
    published: v.optional(v.boolean()),
  },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage projects.");
    }
    const project = await ctx.db.get("Project", args.id);
    if (!project) throw ctx.error("NOT_FOUND", "Project not found.");

    const patch: Record<string, boolean> = {};
    if (typeof args.selected === "boolean") patch.selected = args.selected;
    if (typeof args.published === "boolean") patch.published = args.published;
    if (Object.keys(patch).length > 0) {
      await ctx.db.unsafe.update("Project", args.id, patch);
    }
    return { ok: true };
  },
});
