import { mutation, v } from "@pylonsync/functions";
import { normalizeProjectName } from "../lib/projects";

// updateProject — the reference example for a first-party server function, and
// the core authoring loop end to end: entity → policy → FUNCTION → call it from
// the client. The Projects tab's edit form (app/dashboard/dashboard-client.tsx)
// saves through this via `callFn("updateProject", …)`.
//
// Why a server function and not a direct client `db.update`? Two reasons the
// pattern exists:
//   1. Server-side validation the client can't be trusted to enforce — the name
//      bounds run on the server no matter what the browser sends.
//   2. Authorization beyond "owns the row". Functions BYPASS entity policies and
//      run with full DB access, so a handler that trusted a caller-supplied
//      `projectId` would be an IDOR. `ctx.requireMember` re-checks that the
//      caller belongs to THAT project's workspace, failing CLOSED (throws
//      FORBIDDEN otherwise). Pass `{ role: ["owner", "admin"] }` to make edits
//      admin-only.
//
// Quick archive/delete stay as direct client `db` writes — the Project row
// policy (`auth.tenantId == data.orgId`) already covers "edit a row you own".
// Reach for a function when you need more than that.
export default mutation<
  { projectId: string; name: string; description?: string },
  { id: string; name: string }
>({
  // `auth` defaults to "user" (secure-by-default) — requireMember does the rest.
  args: {
    projectId: v.string(),
    name: v.string(),
    description: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const name = normalizeProjectName(args.name);
    if (!name) {
      throw ctx.error("INVALID_ARGS", "Project name must be 1–80 characters.");
    }
    const description = args.description?.trim() || undefined;

    // Load the target row, then authorize against ITS OWN workspace — never a
    // caller-supplied org id.
    const project = await ctx.db.get("Project", args.projectId);
    if (!project) {
      throw ctx.error("NOT_FOUND", "Project not found.");
    }
    await ctx.requireMember(project.orgId as string);

    // Authorized above, so write through the explicit trusted-handler surface
    // (also correct under PYLON_STRICT_FN_POLICIES).
    await ctx.db.unsafe.update("Project", args.projectId, { name, description });
    return { id: args.projectId, name };
  },
});
