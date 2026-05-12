/**
 * `@pylonsync/organizations` — declarative permission + team layer
 * for Pylon apps using the framework's entity-based org system
 * (v0.3.74+).
 *
 * Usage:
 *
 * ```ts
 * import { organizations } from "@pylonsync/organizations";
 *
 * export const orgs = organizations({
 *   permissions: {
 *     "projects.create": ["owner", "admin"],
 *     "projects.delete": ["owner"],
 *     "members.invite":  ["owner", "admin"],
 *     "billing.manage":  ["owner"],
 *     "teams.create":    ["owner", "admin"],
 *     "teams.members.manage": ["owner", "admin"],
 *   },
 *   teams: { enabled: true, maximumTeamsPerOrg: 50 },
 * });
 *
 * // app.ts:
 * buildManifest({
 *   entities: [Org, OrgMember, OrgInvite, ...orgs.manifest.entities],
 *   policies: [...orgs.manifest.policies],
 *   queries:  [...orgs.manifest.queries],
 *   actions:  [...orgs.manifest.actions],
 * });
 *
 * // In any function handler:
 * import { requirePermission } from "@pylonsync/organizations/runtime";
 * await requirePermission(ctx, orgs.config, "projects.delete");
 * ```
 *
 * The package does NOT redeclare Org / OrgMember / OrgInvite — the
 * framework's v0.3.74+ entity-based org system already owns those.
 * Apps customize the schema directly (add `plan`, `slug`, billing
 * fields, etc.) and point the framework at the entity names via the
 * manifest's `auth.org` block. This plugin layers a permission
 * system + team support on top.
 */

import {
	buildOrganizationsManifest,
	type OrganizationsManifestFragment,
} from "./manifest";
import { internalHandlers } from "./handlers/internals";
import { organizationsQueryHandlers } from "./handlers/queries";
import { teamHandlers } from "./handlers/teams";
import type { OrganizationsConfig } from "./types";

export type {
	OrganizationsConfig,
	TeamsConfig,
	OrgHooks,
	HandlerCtx,
} from "./types";
export type { OrganizationsManifestFragment } from "./manifest";
export {
	requirePermission,
	hasPermission,
	resolveCallerRole,
	rolesAllowed,
	DEFAULT_ROLES,
} from "./permissions";
export {
	orgEntityName,
	memberEntityName,
	inviteEntityName,
	teamEntityName,
	teamMemberEntityName,
} from "./entities";

export interface OrganizationsPlugin {
	config: OrganizationsConfig;
	manifest: OrganizationsManifestFragment;
	handlers: Record<string, unknown>;
}

export function organizations(
	cfg: OrganizationsConfig,
): OrganizationsPlugin {
	return {
		config: cfg,
		manifest: buildOrganizationsManifest(cfg),
		handlers: {
			...organizationsQueryHandlers(cfg),
			...teamHandlers(cfg),
			...internalHandlers(cfg),
		},
	};
}
