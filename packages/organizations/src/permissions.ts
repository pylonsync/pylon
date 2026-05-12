/**
 * Declarative permission system.
 *
 * Apps register a `permissions` map at config time:
 *
 *   permissions: {
 *     "projects.create": ["owner", "admin"],
 *     "projects.delete": ["owner"],
 *     "members.invite": ["owner", "admin"],
 *     "billing.manage": ["owner"],
 *   }
 *
 * And then check permission at action boundaries:
 *
 *   await requirePermission(ctx, cfg, "projects.delete");
 *
 * The check resolves the caller's role for the active tenant, then
 * looks up the role's permitted actions. Replaces the
 * `exists(OrgMember where role = 'owner' or role = 'admin')` policy
 * expression every Pylon app duplicates today.
 *
 * Two reasons this lives client-of-the-framework (TS) rather than
 * inside the policy DSL:
 *   1. Cross-action checks (`requirePermission` inside an action
 *      handler) are easier to reason about than DSL chains.
 *   2. The same map drives client-side rendering: the React `usePermission`
 *      hook reads the same config so UI gating + server enforcement
 *      stay in sync.
 */

import type { HandlerCtx, OrganizationsConfig } from "./types";
import { memberEntityName } from "./entities";

/**
 * Resolve the caller's role in the active tenant. Returns `null`
 * when the caller has no membership row (anonymous, or a logged-in
 * user who isn't in the org). Cached per-request via a `Map`-backed
 * memo that the runtime resets between requests; we approximate by
 * relying on the underlying `runQuery` being fast (one indexed
 * lookup).
 */
export async function resolveCallerRole(
	ctx: HandlerCtx,
	cfg: OrganizationsConfig,
	orgId?: string,
): Promise<string | null> {
	const userId = ctx.auth.userId;
	if (!userId) return null;
	const tenantId = orgId ?? ctx.auth.tenantId;
	if (!tenantId) return null;
	const rows = await ctx.runQuery<Array<{ role: string }>>(
		"_pylonOrgsMembership",
		{ entity: memberEntityName(cfg), orgId: tenantId, userId },
	);
	return rows[0]?.role ?? null;
}

/**
 * Throw `FORBIDDEN` unless the caller's active-tenant role grants
 * the named permission. Admin contexts (`ctx.auth.isAdmin`) bypass
 * the check entirely — they represent platform staff or system
 * automation, not org members.
 */
export async function requirePermission(
	ctx: HandlerCtx,
	cfg: OrganizationsConfig,
	permission: string,
	orgId?: string,
): Promise<void> {
	if (ctx.auth.isAdmin) return;
	if (!ctx.auth.userId) {
		throw ctx.error("UNAUTHENTICATED", "log in to perform this action");
	}
	const allowedRoles = cfg.permissions?.[permission];
	if (!allowedRoles || allowedRoles.length === 0) {
		// No mapping → conservative deny. Apps that forget to declare
		// a permission shouldn't accidentally grant it to anyone.
		throw ctx.error(
			"UNKNOWN_PERMISSION",
			`permission "${permission}" is not declared`,
		);
	}
	const role = await resolveCallerRole(ctx, cfg, orgId);
	if (!role) {
		throw ctx.error("NOT_A_MEMBER", "not a member of the active organization");
	}
	if (!allowedRoles.includes(role)) {
		throw ctx.error(
			"FORBIDDEN",
			`role "${role}" cannot ${permission} (allowed: ${allowedRoles.join(", ")})`,
		);
	}
}

/**
 * Non-throwing variant. Returns `true` when permitted, `false`
 * otherwise. Used by the `hasPermission` query so client code can
 * check + render conditionally without a try/catch.
 */
export async function hasPermission(
	ctx: HandlerCtx,
	cfg: OrganizationsConfig,
	permission: string,
	orgId?: string,
): Promise<boolean> {
	if (ctx.auth.isAdmin) return true;
	if (!ctx.auth.userId) return false;
	const allowed = cfg.permissions?.[permission];
	if (!allowed || allowed.length === 0) return false;
	const role = await resolveCallerRole(ctx, cfg, orgId);
	if (!role) return false;
	return allowed.includes(role);
}

/** Default role catalog matching the framework's three built-in roles. */
export const DEFAULT_ROLES = ["owner", "admin", "member"] as const;

export function rolesAllowed(cfg: OrganizationsConfig): string[] {
	return cfg.roles ?? [...DEFAULT_ROLES];
}
