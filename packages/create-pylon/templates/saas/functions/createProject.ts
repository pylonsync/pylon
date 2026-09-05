import { mutation, v } from "@pylonsync/functions";
import { normalizeProjectName } from "../lib/projects";
import { canCreateProject, planFromSubscription } from "../lib/plans";

// createProject — the free-tier cap lives here, on the server. The Project
// policy denies client inserts, so a browser cannot skip the paywall by
// writing the row itself. Returns LIMIT_REACHED when the workspace is at the
// cap; the Projects tab opens the upgrade dialog on that code.
export default mutation<
  { orgId: string; name: string; description?: string },
  { id: string }
>({
  args: {
    orgId: v.string(),
    name: v.string(),
    description: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const name = normalizeProjectName(args.name);
    if (!name) {
      throw ctx.error("INVALID_ARGS", "Project name must be 1–80 characters.");
    }
    // Any member of the workspace may create a project. Fails closed.
    await ctx.requireMember(args.orgId);

    const subs = (await ctx.db.unsafe.list("StripeSubscription")) as Array<{
      referenceId: string;
      plan?: string;
      status?: string;
    }>;
    const sub = subs.find((s) => s.referenceId === args.orgId) ?? null;
    const plan = planFromSubscription(sub);

    const projects = (await ctx.db.unsafe.list("Project")) as Array<{
      orgId: string;
      status?: string;
    }>;
    const active = projects.filter(
      (p) => p.orgId === args.orgId && (p.status ?? "active") !== "archived",
    ).length;
    if (!canCreateProject(plan, active)) {
      throw ctx.error(
        "LIMIT_REACHED",
        "This workspace is at the free plan's project limit. Upgrade to Pro for unlimited projects.",
      );
    }

    const id = (await ctx.db.unsafe.insert("Project", {
      orgId: args.orgId,
      name,
      status: "active",
      ...(args.description?.trim() ? { description: args.description.trim() } : {}),
    })) as string;
    return { id };
  },
});
