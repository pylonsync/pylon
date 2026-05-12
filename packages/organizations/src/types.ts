/**
 * `@pylonsync/organizations` — a declarative permission + team layer
 * on top of Pylon's framework org primitives (v0.3.74+).
 *
 * The framework already provides:
 *   - Org / OrgMember / OrgInvite as manifest entities
 *   - /api/auth/orgs/* routes (create/list/get/delete/members/invites)
 *   - /api/auth/select-org for active-tenant selection
 *   - `ctx.auth.tenantId` for tenant-scoped policies
 *   - TenantScopePlugin auto-stamps tenantId on inserts
 *
 * What this package adds:
 *   - **Permission system**: declarative `permissions: { "x.y": ["owner", "admin"] }`
 *     config + a `requirePermission(ctx, "x.y")` helper. Replaces the
 *     verbose `exists(OrgMember where ...)` policy expressions every
 *     app duplicates across every entity.
 *   - **Team support**: optional Team + TeamMember entities + helpers,
 *     so a customer's org can have sub-groups (engineering, marketing).
 *   - **Audit-aware invite/role mutations**: wrap the framework's
 *     `/api/auth/orgs/:id/members` + `/invites` flows with optional
 *     hooks that fire on every member/role/invite change. Apps wire
 *     these into @pylonsync/audit-log for compliance trails.
 *   - **React hooks** (in a separate peer module): useActiveOrg,
 *     useOrgMembers, usePermission, useTeams.
 */

export interface OrganizationsConfig {
	/**
	 * Permission catalog. Map of permission name → array of role names
	 * that grant the permission. The framework's three default roles
	 * are `owner`, `admin`, `member`; apps can add custom ones via the
	 * org's role assignment flow.
	 *
	 * Convention: dot-separated namespaces (`projects.create`,
	 * `billing.manage`, `members.invite`). Apps that prefer a flatter
	 * naming can use whatever they want — the system treats them as
	 * opaque strings.
	 */
	permissions?: Record<string, string[]>;
	/**
	 * Roles allowed at all. Defaults to `["owner", "admin", "member"]`
	 * to match the framework. Apps that want custom roles
	 * (`billing_admin`, `viewer`, etc.) override this; the
	 * `setMemberRole` mutation rejects roles not in this list.
	 */
	roles?: string[];
	/**
	 * Enable team support. When true, the manifest fragment adds
	 * `Team` + `TeamMember` entities and exposes team-management
	 * actions (`createTeam`, `addTeamMember`, etc.). Off by default
	 * to keep the manifest small for apps that don't need teams.
	 */
	teams?: TeamsConfig;
	/**
	 * Override entity names. Defaults match the framework's
	 * conventions: `Org`, `OrgMember`, `OrgInvite`. Cloud uses
	 * `Organization` — set `org: "Organization"` here.
	 */
	entities?: {
		org?: string;
		member?: string;
		invite?: string;
		team?: string;
		teamMember?: string;
	};
	/** Lifecycle hooks fired after every member/role/invite change. */
	hooks?: OrgHooks;
	/**
	 * Cap on members per org. The framework doesn't enforce this;
	 * the wrapper actions check it before adding. Apps that bill
	 * per-seat want a hard cap; SaaS that bills usage-based can
	 * leave it `Infinity`.
	 */
	membershipLimit?: number;
	/**
	 * Cap on pending invites per org. Prevents an admin from
	 * spamming the invite endpoint to email-bomb random addresses.
	 */
	invitationLimit?: number;
}

export interface TeamsConfig {
	enabled: boolean;
	maximumTeamsPerOrg?: number;
	maximumMembersPerTeam?: number;
}

export interface OrgHooks {
	onMemberAdd?: (
		ctx: HandlerCtx,
		args: { orgId: string; userId: string; role: string },
	) => Promise<void> | void;
	onMemberRemove?: (
		ctx: HandlerCtx,
		args: { orgId: string; userId: string; role: string },
	) => Promise<void> | void;
	onRoleChange?: (
		ctx: HandlerCtx,
		args: {
			orgId: string;
			userId: string;
			oldRole: string;
			newRole: string;
		},
	) => Promise<void> | void;
	onInviteCreate?: (
		ctx: HandlerCtx,
		args: { orgId: string; email: string; role: string; inviteId: string },
	) => Promise<void> | void;
	onInviteAccept?: (
		ctx: HandlerCtx,
		args: { orgId: string; userId: string; role: string; inviteId: string },
	) => Promise<void> | void;
	onInviteRevoke?: (
		ctx: HandlerCtx,
		args: { orgId: string; inviteId: string },
	) => Promise<void> | void;
	onTeamCreate?: (
		ctx: HandlerCtx,
		args: { orgId: string; teamId: string; name: string },
	) => Promise<void> | void;
}

/**
 * Subset of ActionCtx the wrapper handlers need. Keeps the package
 * decoupled from `@pylonsync/functions`' full ctx type.
 */
export interface HandlerCtx {
	env: Record<string, string | undefined>;
	auth: { userId?: string | null; tenantId?: string | null; isAdmin?: boolean };
	runQuery: <T>(name: string, args: Record<string, unknown>) => Promise<T>;
	runMutation: <T = unknown>(
		name: string,
		args: Record<string, unknown>,
	) => Promise<T>;
	error: (code: string, message: string) => Error;
}
