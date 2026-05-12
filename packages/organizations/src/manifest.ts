import {
	type ActionDefinition,
	type EntityDefinition,
	type PolicyDefinition,
	type QueryDefinition,
	action,
	entity,
	field,
	policy,
	query,
} from "@pylonsync/sdk";

import { teamEntityName, teamMemberEntityName } from "./entities";
import type { OrganizationsConfig } from "./types";

export interface OrganizationsManifestFragment {
	entities: EntityDefinition[];
	policies: PolicyDefinition[];
	queries: QueryDefinition[];
	actions: ActionDefinition[];
}

/**
 * Manifest fragment. Returns:
 *
 *   - Optional Team + TeamMember entities (when teams.enabled)
 *   - Policies that tenant-scope teams to their owning org
 *   - Action declarations for the wrapped invite/role mgmt + team mgmt
 *   - Query declarations for permission checks + member listing
 *
 * The Org / OrgMember / OrgInvite entities themselves are NOT
 * declared here — those live on the app side (per framework v0.3.74
 * convention), and the plugin reads them via the entity-name
 * config. This keeps schemas customizable: an app that wants to
 * call its tenant entity `Workspace` doesn't have to fork the plugin.
 */
export function buildOrganizationsManifest(
	cfg: OrganizationsConfig,
): OrganizationsManifestFragment {
	const entities: EntityDefinition[] = [];
	const policies: PolicyDefinition[] = [];
	const queries: QueryDefinition[] = [
		query("orgPermissions"),
		query("orgRoleOf", {
			input: [{ name: "orgId", type: "string", optional: true }],
		}),
		query("orgHasPermission", {
			input: [
				{ name: "permission", type: "string" },
				{ name: "orgId", type: "string", optional: true },
			],
		}),
	];
	const actions: ActionDefinition[] = [];

	if (cfg.teams?.enabled) {
		const teamEnt = teamEntityName(cfg);
		const teamMemberEnt = teamMemberEntityName(cfg);
		entities.push(
			entity(teamEnt, {
				orgId: field.string(),
				name: field.string(),
				slug: field.string(),
				description: field.string().optional(),
				createdBy: field.string(),
				createdAt: field.string(),
			}),
			entity(teamMemberEnt, {
				teamId: field.string(),
				userId: field.string(),
				orgId: field.string(),
				role: field.string(),
				joinedAt: field.string(),
			}),
		);
		policies.push(
			policy({
				name: `${teamEnt.toLowerCase()}_tenant_scoped`,
				entity: teamEnt,
				allowRead: "auth.tenantId == data.orgId",
				allowInsert: "false",
				allowUpdate: "false",
				allowDelete: "false",
			}),
			policy({
				name: `${teamMemberEnt.toLowerCase()}_tenant_scoped`,
				entity: teamMemberEnt,
				allowRead: "auth.tenantId == data.orgId",
				allowInsert: "false",
				allowUpdate: "false",
				allowDelete: "false",
			}),
		);
		actions.push(
			action("createTeam", {
				input: [
					{ name: "name", type: "string" },
					{ name: "slug", type: "string", optional: true },
					{ name: "description", type: "string", optional: true },
				],
			}),
			action("deleteTeam", {
				input: [{ name: "teamId", type: "string" }],
			}),
			action("addTeamMember", {
				input: [
					{ name: "teamId", type: "string" },
					{ name: "userId", type: "string" },
					{ name: "role", type: "string", optional: true },
				],
			}),
			action("removeTeamMember", {
				input: [
					{ name: "teamId", type: "string" },
					{ name: "userId", type: "string" },
				],
			}),
		);
	}

	return { entities, policies, queries, actions };
}
