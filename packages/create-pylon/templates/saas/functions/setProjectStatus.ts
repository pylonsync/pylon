import { mutation, v } from "@pylonsync/functions";
import { canCreateProject, planFromSubscriptions } from "../lib/plans";

// setProjectStatus — archives or restores a project. Restoring counts
// against the free plan's active-project cap the same way createProject
// does, so archiving and restoring cannot get a free workspace past it.
export default mutation<{ projectId: string; status: string }, { id: string; status: string }>({
  args: { projectId: v.string(), status: v.string() },
  async handler(ctx, args) {
    if (args.status !== "active" && args.status !== "archived") {
      throw ctx.error("INVALID_ARGS", "Status must be active or archived.");
    }
    const project = await ctx.db.unsafe.get("Project", args.projectId);
    if (!project) throw ctx.error("NOT_FOUND", "Project not found.");
    const orgId = String(project.orgId);
    await ctx.requireMember(orgId);

    const current = (project.status as string | undefined) ?? "active";
    if (current === args.status) return { id: args.projectId, status: current };

    if (args.status === "active") {
      const subs = (await ctx.db.unsafe.query("StripeSubscription", {
        referenceId: orgId,
      })) as Array<{ plan?: string; status?: string }>;
      const plan = planFromSubscriptions(subs);
      const projects = (await ctx.db.unsafe.query("Project", { orgId })) as Array<{
        status?: string;
      }>;
      const active = projects.filter((p) => (p.status ?? "active") !== "archived").length;
      if (!canCreateProject(plan, active)) {
        throw ctx.error(
          "LIMIT_REACHED",
          "This workspace is at the free plan's project limit. Upgrade to Pro for unlimited projects.",
        );
      }
    }

    await ctx.db.unsafe.update("Project", args.projectId, { status: args.status });
    return { id: args.projectId, status: args.status };
  },
});
