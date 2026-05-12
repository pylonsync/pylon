import { mutation, query, v } from "@pylonsync/functions";

import { teamEntityName, teamMemberEntityName } from "../entities";
import type { OrganizationsConfig } from "../types";

/**
 * Plugin-internal queries + mutations for team mgmt. Same naming
 * convention as @pylonsync/stripe — `_pylonOrgs*` prefix +
 * `internal: true` to keep them off the public HTTP surface.
 */
export function internalHandlers(
	cfg: OrganizationsConfig,
): Record<string, unknown> {
	if (!cfg.teams?.enabled) return {};
	const teamEnt = teamEntityName(cfg);
	const teamMemberEnt = teamMemberEntityName(cfg);

	return {
		_pylonOrgsListTeams: query({
			args: { orgId: v.string() },
			internal: true,
			async handler(ctx, args: { orgId: string }) {
				return ctx.db.query(teamEnt, { orgId: args.orgId });
			},
		}),

		_pylonOrgsListTeamMembers: query({
			args: { teamId: v.string() },
			internal: true,
			async handler(ctx, args: { teamId: string }) {
				return ctx.db.query(teamMemberEnt, { teamId: args.teamId });
			},
		}),

		_pylonOrgsInsertTeam: mutation({
			args: {
				orgId: v.string(),
				name: v.string(),
				slug: v.string(),
				description: v.optional(v.string()),
				createdBy: v.string(),
				createdAt: v.string(),
			},
			internal: true,
			async handler(
				ctx,
				args: {
					orgId: string;
					name: string;
					slug: string;
					description?: string | null;
					createdBy: string;
					createdAt: string;
				},
			) {
				return ctx.db.insert(teamEnt, {
					orgId: args.orgId,
					name: args.name,
					slug: args.slug,
					description: args.description ?? null,
					createdBy: args.createdBy,
					createdAt: args.createdAt,
				});
			},
		}),

		_pylonOrgsDeleteTeam: mutation({
			args: { teamId: v.string() },
			internal: true,
			async handler(ctx, args: { teamId: string }) {
				// Cascade-delete members first. We don't use the framework's
				// cascade primitive because it's only wired for entity-API
				// writes, not direct ctx.db calls.
				const members = (await ctx.db.query(teamMemberEnt, {
					teamId: args.teamId,
				})) as Array<{ id: string }>;
				for (const m of members) {
					await ctx.db.delete(teamMemberEnt, m.id);
				}
				await ctx.db.delete(teamEnt, args.teamId);
			},
		}),

		_pylonOrgsInsertTeamMember: mutation({
			args: {
				teamId: v.string(),
				userId: v.string(),
				orgId: v.string(),
				role: v.string(),
				joinedAt: v.string(),
			},
			internal: true,
			async handler(
				ctx,
				args: {
					teamId: string;
					userId: string;
					orgId: string;
					role: string;
					joinedAt: string;
				},
			) {
				// Idempotent: re-adding an existing member is a no-op so
				// retries (network blips, double-clicks) don't insert
				// duplicates.
				const existing = (await ctx.db.query(teamMemberEnt, {
					teamId: args.teamId,
					userId: args.userId,
				})) as Array<{ id: string }>;
				if (existing[0]) return existing[0].id;
				return ctx.db.insert(teamMemberEnt, {
					teamId: args.teamId,
					userId: args.userId,
					orgId: args.orgId,
					role: args.role,
					joinedAt: args.joinedAt,
				});
			},
		}),

		_pylonOrgsRemoveTeamMember: mutation({
			args: { teamId: v.string(), userId: v.string() },
			internal: true,
			async handler(ctx, args: { teamId: string; userId: string }) {
				const rows = (await ctx.db.query(teamMemberEnt, {
					teamId: args.teamId,
					userId: args.userId,
				})) as Array<{ id: string }>;
				for (const r of rows) {
					await ctx.db.delete(teamMemberEnt, r.id);
				}
			},
		}),
	};
}
