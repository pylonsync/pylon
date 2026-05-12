import { query, v } from "@pylonsync/functions";

import { memberEntityName } from "../entities";
import { hasPermission, resolveCallerRole } from "../permissions";
import type { HandlerCtx, OrganizationsConfig } from "../types";

/**
 * Permission + membership queries the app exposes to the frontend.
 *
 *   - `orgPermissions`  → flat permission list the caller's role
 *                          grants in the active tenant. Drives client-
 *                          side rendering (`{permissions.includes('x.y')
 *                          ? <Button/> : null}`) without per-button
 *                          round trips.
 *   - `orgRoleOf`        → caller's role string in the (optionally
 *                          specified) org. Returns null when the
 *                          caller isn't a member.
 *   - `orgHasPermission` → single-permission check, server-side
 *                          authoritative. Use when the frontend
 *                          needs a defensive check before a costly
 *                          server action.
 *
 * Plus internal queries used by the wrappers themselves.
 */
export function organizationsQueryHandlers(
	cfg: OrganizationsConfig,
): Record<string, unknown> {
	return {
		orgPermissions: query({
			args: { orgId: v.optional(v.string()) },
			async handler(
				ctx,
				args: { orgId?: string },
			) {
				const role = await resolveCallerRole(
					{
						auth: ctx.auth,
						env: ctx.env,
						error: (c, m) => new Error(`${c}: ${m}`),
						runQuery: (async (_name, qargs) =>
							ctx.db.query(qargs.entity as string, {
								orgId: qargs.orgId,
								userId: qargs.userId,
							})) as HandlerCtx["runQuery"],
						runMutation: async () => {
							throw new Error("not supported in query ctx");
						},
					},
					cfg,
					args.orgId,
				);
				if (!role) return [];
				const out: string[] = [];
				for (const [perm, roles] of Object.entries(cfg.permissions ?? {})) {
					if (roles.includes(role)) out.push(perm);
				}
				return out;
			},
		}),

		orgRoleOf: query({
			args: { orgId: v.optional(v.string()) },
			async handler(ctx, args: { orgId?: string }) {
				const tenantId = args.orgId ?? ctx.auth.tenantId;
				if (!tenantId || !ctx.auth.userId) return null;
				const rows = (await ctx.db.query(memberEntityName(cfg), {
					orgId: tenantId,
					userId: ctx.auth.userId,
				})) as Array<{ role: string }>;
				return rows[0]?.role ?? null;
			},
		}),

		orgHasPermission: query({
			args: {
				permission: v.string(),
				orgId: v.optional(v.string()),
			},
			async handler(
				ctx,
				args: { permission: string; orgId?: string },
			) {
				return hasPermission(
					{
						auth: ctx.auth,
						env: ctx.env,
						error: (c, m) => new Error(`${c}: ${m}`),
						runQuery: (async (_name, qargs) =>
							ctx.db.query(qargs.entity as string, {
								orgId: qargs.orgId,
								userId: qargs.userId,
							})) as HandlerCtx["runQuery"],
						runMutation: async () => {
							throw new Error("not supported in query ctx");
						},
					},
					cfg,
					args.permission,
					args.orgId,
				);
			},
		}),

		// Internal — used by requirePermission inside action handlers.
		_pylonOrgsMembership: query({
			args: {
				entity: v.string(),
				orgId: v.string(),
				userId: v.string(),
			},
			internal: true,
			async handler(
				ctx,
				args: { entity: string; orgId: string; userId: string },
			) {
				return ctx.db.query(args.entity, {
					orgId: args.orgId,
					userId: args.userId,
				});
			},
		}),
	};
}
