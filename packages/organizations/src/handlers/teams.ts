import { action, v } from "@pylonsync/functions";

import { teamEntityName, teamMemberEntityName } from "../entities";
import { requirePermission } from "../permissions";
import type { HandlerCtx, OrganizationsConfig } from "../types";

/**
 * Team management handlers. Only registered when
 * `cfg.teams.enabled === true`.
 *
 * Permission convention (apps customize via `cfg.permissions`):
 *   - `teams.create`         → createTeam, deleteTeam
 *   - `teams.members.manage` → addTeamMember, removeTeamMember
 *
 * Apps that don't declare those permissions will get
 * `UNKNOWN_PERMISSION` on every team action — the conservative
 * deny in `requirePermission` exists exactly to catch this footgun.
 */
export function teamHandlers(
	cfg: OrganizationsConfig,
): Record<string, unknown> {
	if (!cfg.teams?.enabled) return {};
	const teamEnt = teamEntityName(cfg);
	const teamMemberEnt = teamMemberEntityName(cfg);
	const slugify = (s: string) =>
		s
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, "-")
			.replace(/^-+|-+$/g, "")
			.slice(0, 50);

	const maxTeams = cfg.teams.maximumTeamsPerOrg ?? Number.POSITIVE_INFINITY;
	const maxPerTeam =
		cfg.teams.maximumMembersPerTeam ?? Number.POSITIVE_INFINITY;

	return {
		createTeam: action({
			args: {
				name: v.string(),
				slug: v.optional(v.string()),
				description: v.optional(v.string()),
			},
			async handler(
				ctx,
				args: { name: string; slug?: string; description?: string },
			) {
				const orgId = ctx.auth.tenantId;
				if (!orgId) throw ctx.error("NO_ORG", "select an org first");
				await requirePermission(
					ctx as unknown as HandlerCtx,
					cfg,
					"teams.create",
				);

				const existing = (await ctx.runQuery("_pylonOrgsListTeams", {
					orgId,
				})) as Array<unknown>;
				if (existing.length >= maxTeams) {
					throw ctx.error(
						"TEAM_LIMIT",
						`org has reached its team limit (${maxTeams})`,
					);
				}

				const teamId = await ctx.runMutation<string>("_pylonOrgsInsertTeam", {
					orgId,
					name: args.name,
					slug: args.slug ? slugify(args.slug) : slugify(args.name),
					description: args.description ?? null,
					createdBy: ctx.auth.userId ?? "",
					createdAt: new Date().toISOString(),
				});

				if (cfg.hooks?.onTeamCreate) {
					await cfg.hooks.onTeamCreate(ctx as unknown as HandlerCtx, {
						orgId,
						teamId,
						name: args.name,
					});
				}
				return { teamId };
			},
		}),

		deleteTeam: action({
			args: { teamId: v.string() },
			async handler(ctx, args: { teamId: string }) {
				await requirePermission(
					ctx as unknown as HandlerCtx,
					cfg,
					"teams.create",
				);
				await ctx.runMutation("_pylonOrgsDeleteTeam", {
					teamId: args.teamId,
				});
				return { ok: true };
			},
		}),

		addTeamMember: action({
			args: {
				teamId: v.string(),
				userId: v.string(),
				role: v.optional(v.string()),
			},
			async handler(
				ctx,
				args: { teamId: string; userId: string; role?: string },
			) {
				const orgId = ctx.auth.tenantId;
				if (!orgId) throw ctx.error("NO_ORG", "select an org first");
				await requirePermission(
					ctx as unknown as HandlerCtx,
					cfg,
					"teams.members.manage",
				);

				const members = (await ctx.runQuery("_pylonOrgsListTeamMembers", {
					teamId: args.teamId,
				})) as Array<unknown>;
				if (members.length >= maxPerTeam) {
					throw ctx.error(
						"TEAM_MEMBER_LIMIT",
						`team has reached its member limit (${maxPerTeam})`,
					);
				}

				await ctx.runMutation("_pylonOrgsInsertTeamMember", {
					teamId: args.teamId,
					userId: args.userId,
					orgId,
					role: args.role ?? "member",
					joinedAt: new Date().toISOString(),
				});
				return { ok: true };
			},
		}),

		removeTeamMember: action({
			args: { teamId: v.string(), userId: v.string() },
			async handler(ctx, args: { teamId: string; userId: string }) {
				await requirePermission(
					ctx as unknown as HandlerCtx,
					cfg,
					"teams.members.manage",
				);
				await ctx.runMutation("_pylonOrgsRemoveTeamMember", {
					teamId: args.teamId,
					userId: args.userId,
				});
				return { ok: true };
			},
		}),
	};
}
