import {
	entity,
	field,
	query,
	action,
	policy,
	buildManifest,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Multi-tenant SaaS schema. The shape:
//
//   User       — global, one per real person (auth-managed)
//   Org        — a tenant. Has a unique slug. Owner is the founding User.
//   Membership — pivot row: which Users belong to which Orgs, and at
//                what role (owner | admin | member).
//   Project    — an org-scoped resource. Every row belongs to one Org;
//                policies enforce that a query without `orgId` returns
//                nothing.
//
// Pylon's policy expression language can read `auth.userId` and any
// columns on the row. Cross-row checks (e.g. "is the caller a member
// of this org?") are expressed by joining a Membership probe into the
// allowRead/allowInsert/etc. expression.
// ---------------------------------------------------------------------------

const Org = entity("Org", {
	slug: field.string(),
	name: field.string(),
	// `.readonly()` blocks HTTP PATCH from rewriting ownership. Server-side
	// ctx.db.update still goes through, so admin migrations + transfers work.
	ownerId: field.id("User").readonly(),
	createdAt: field.datetime().readonly(),
});

const Membership = entity("Membership", {
	orgId: field.id("Org").readonly(),
	userId: field.id("User").readonly(),
	role: field.enum_(["owner", "admin", "member"]),
	createdAt: field.datetime().readonly(),
});

const Project = entity("Project", {
	orgId: field.id("Org").readonly(),
	name: field.string(),
	createdBy: field.id("User").readonly(),
	archived: field.bool(),
	createdAt: field.datetime().readonly(),
});

// ---------------------------------------------------------------------------
// Function declarations
// ---------------------------------------------------------------------------

const myOrgs = query("myOrgs");
const orgMembers = query("orgMembers");
const orgProjects = query("orgProjects");

const createOrg = action("createOrg", {
	input: [
		{ name: "slug", type: "string" },
		{ name: "name", type: "string" },
	],
});

const inviteMember = action("inviteMember", {
	input: [
		{ name: "orgId", type: "id(Org)" },
		{ name: "userId", type: "id(User)" },
		{ name: "role", type: "string" },
	],
});

const removeMember = action("removeMember", {
	input: [
		{ name: "orgId", type: "id(Org)" },
		{ name: "userId", type: "id(User)" },
	],
});

const setMemberRole = action("setMemberRole", {
	input: [
		{ name: "orgId", type: "id(Org)" },
		{ name: "userId", type: "id(User)" },
		{ name: "role", type: "string" },
	],
});

const createProject = action("createProject", {
	input: [
		{ name: "orgId", type: "id(Org)" },
		{ name: "name", type: "string" },
	],
});

const archiveProject = action("archiveProject", {
	input: [{ name: "id", type: "id(Project)" }],
});

// ---------------------------------------------------------------------------
// Policies — RBAC enforced on every read/write.
//
// Each entity's policy hooks read `auth.userId` and check membership
// of the row's `orgId` via the Membership table. Pylon evaluates
// policy expressions against the database, so a `Membership(orgId,
// userId)` probe is a join the planner pushes into the same query —
// no per-request round trip.
// ---------------------------------------------------------------------------

const orgPolicy = policy({
	name: "org_membership",
	entity: "Org",
	// Anyone can read an org if they're a member; only the owner can
	// update / delete. Update + delete pin `existing.ownerId` (the row's
	// current value) not `data.ownerId` (the payload), so an attacker
	// can't PATCH `{ownerId: <them>}` past the check.
	allowRead: "exists(Membership where orgId = data.id and userId = auth.userId)",
	allowInsert: "auth.userId == data.ownerId",
	allowUpdate: "existing.ownerId == auth.userId",
	allowDelete: "existing.ownerId == auth.userId",
});

const membershipPolicy = policy({
	name: "membership_admin",
	entity: "Membership",
	// You can see your own memberships, plus all memberships in any org
	// where you're an owner/admin (so the admin UI can list everyone).
	// Update/delete pin `existing.orgId` (not the payload); `orgId` +
	// `userId` are `.readonly()` too, so `setMemberRole` writes only `role`.
	allowRead:
		"data.userId == auth.userId or exists(Membership where orgId = data.orgId and userId = auth.userId and (role = 'owner' or role = 'admin'))",
	allowInsert:
		"exists(Membership where orgId = data.orgId and userId = auth.userId and (role = 'owner' or role = 'admin'))",
	allowUpdate:
		"exists(Membership where orgId = existing.orgId and userId = auth.userId and (role = 'owner' or role = 'admin'))",
	allowDelete:
		"exists(Membership where orgId = existing.orgId and userId = auth.userId and (role = 'owner' or role = 'admin'))",
});

const projectPolicy = policy({
	name: "project_org_scope",
	entity: "Project",
	// Tenant scope: you can only touch a project if you're a member of
	// its org. Update/delete pin `existing.orgId` (not the payload) so a
	// project can't be "imported" into a foreign org; insert uses
	// `data.orgId` since there's no `existing` row yet.
	allowRead:
		"exists(Membership where orgId = data.orgId and userId = auth.userId)",
	allowInsert:
		"exists(Membership where orgId = data.orgId and userId = auth.userId)",
	// Only owners/admins can rename or archive.
	allowUpdate:
		"exists(Membership where orgId = existing.orgId and userId = auth.userId and (role = 'owner' or role = 'admin'))",
	allowDelete:
		"exists(Membership where orgId = existing.orgId and userId = auth.userId and (role = 'owner' or role = 'admin'))",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
	name: "__APP_NAME_SNAKE__",
	version: "0.0.1",
	entities: [Org, Membership, Project],
	queries: [myOrgs, orgMembers, orgProjects],
	actions: [
		createOrg,
		inviteMember,
		removeMember,
		setMemberRole,
		createProject,
		archiveProject,
	],
	policies: [orgPolicy, membershipPolicy, projectPolicy],
	routes: [],
});

console.log(JSON.stringify(manifest));
