import { expect, test } from "bun:test";
import { makeRequireMember } from "./member";
// Templates resolve packages after scaffolding. Load their source with the local SDK.
async function template(name: string) {
  const file = new URL(`../../create-pylon/templates/backend/b2b/apps/api/functions/${name}.ts`, import.meta.url);
  const source = (await Bun.file(file).text()).replace('"@pylonsync/functions"', JSON.stringify(new URL("./index.ts", import.meta.url).pathname));
  const code = new Bun.Transpiler({ loader: "ts" }).transformSync(source);
  return (await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`)).default;
}

const inviteMember = await template("inviteMember");
const setMemberRole = await template("setMemberRole");
const removeMember = await template("removeMember");
const archiveProject = await template("archiveProject");
const createProject = await template("createProject");
const orgMembers = await template("orgMembers");
const orgProjects = await template("orgProjects");
const createOrg = await template("createOrg");

type Row = Record<string, unknown> & { id: string };

// The database has no policy filter. Each public handler must authorize access.
function fixture(userId: string) {
  const rows: Record<string, Row[]> = {
    Org: [{ id: "org-a", ownerId: "owner" }],
    Membership: [
      { id: "m-owner", orgId: "org-a", userId: "owner", role: "owner" },
      { id: "m-admin", orgId: "org-a", userId: "admin", role: "admin" },
      { id: "m-member", orgId: "org-a", userId: "member", role: "member" },
      { id: "m-foreign", orgId: "org-b", userId: "foreign", role: "owner" },
    ],
    Project: [{ id: "project-a", orgId: "org-a", archived: false }],
  };
  const db = {
    async query(entity: string, filter: Record<string, unknown>) {
      return rows[entity].filter(row => Object.entries(filter).every(([key, value]) => row[key] === value));
    },
    async get(entity: string, id: string) {
      return rows[entity].find(row => row.id === id) ?? null;
    },
    async insert(entity: string, data: Record<string, unknown>) {
      const id = `new-${rows[entity].length}`;
      rows[entity].push({ ...data, id });
      return id;
    },
    async update(entity: string, id: string, data: Record<string, unknown>) {
      Object.assign(rows[entity].find(row => row.id === id)!, data);
    },
    async delete(entity: string, id: string) {
      rows[entity] = rows[entity].filter(row => row.id !== id);
    },
  };
  const ctx = {
    auth: { userId },
    db: { ...db, unsafe: db },
    requireMember: makeRequireMember(userId, db.query),
    error: (code: string, message: string) => Object.assign(new Error(message), { code }),
  };
  return { ctx, rows };
}

const adminCalls = [
  (ctx: never) => inviteMember.handler(ctx, { orgId: "org-a", userId: "new-user", role: "owner" }),
  (ctx: never) => setMemberRole.handler(ctx, { orgId: "org-a", userId: "member", role: "owner" }),
  (ctx: never) => removeMember.handler(ctx, { orgId: "org-a", userId: "member" }),
  (ctx: never) => archiveProject.handler(ctx, { id: "project-a" }),
];

for (const user of ["outsider", "member", "foreign"]) {
  test(`${user} cannot change organization access or archive projects`, async () => {
    for (const call of adminCalls) {
      const { ctx, rows } = fixture(user);
      const before = structuredClone(rows);
      await expect(call(ctx as never)).rejects.toMatchObject({ code: "FORBIDDEN" });
      expect(rows).toEqual(before);
    }
  });
}

for (const user of ["owner", "admin"]) {
  test(`${user} can manage organization members and projects`, async () => {
    for (const call of adminCalls) {
      const { ctx, rows } = fixture(user);
      const before = structuredClone(rows);
      await call(ctx as never);
      expect(rows).not.toEqual(before);
    }
  });
}

for (const user of ["outsider", "foreign"]) {
  test(`${user} cannot read organization data or create a project`, async () => {
    const { ctx, rows } = fixture(user);
    const before = structuredClone(rows);
    await expect(orgMembers.handler(ctx as never, { orgId: "org-a" })).rejects.toMatchObject({ code: "FORBIDDEN" });
    await expect(orgProjects.handler(ctx as never, { orgId: "org-a" })).rejects.toMatchObject({ code: "FORBIDDEN" });
    await expect(createProject.handler(ctx as never, { orgId: "org-a", name: "Injected" })).rejects.toMatchObject({ code: "FORBIDDEN" });
    expect(rows).toEqual(before);
  });
}

test("members can read projects and their own membership, and create projects", async () => {
  const { ctx } = fixture("member");
  expect(await orgMembers.handler(ctx as never, { orgId: "org-a" })).toEqual([
    { id: "m-member", orgId: "org-a", userId: "member", role: "member" },
  ]);
  expect(await orgProjects.handler(ctx as never, { orgId: "org-a" })).toHaveLength(1);
  expect(await createProject.handler(ctx as never, { orgId: "org-a", name: "New" })).toMatchObject({ createdBy: "member", orgId: "org-a" });
});

test("administrators can list the organization's members", async () => {
  const { ctx } = fixture("admin");
  expect(await orgMembers.handler(ctx as never, { orgId: "org-a" })).toHaveLength(3);
});

test("organization creation grants the founding membership only to the caller", async () => {
  const { ctx, rows } = fixture("outsider");
  const org = await createOrg.handler(ctx as never, { slug: "new-org", name: "New org" });
  expect(org).toMatchObject({ ownerId: "outsider" });
  expect(rows.Membership.at(-1)).toMatchObject({ orgId: org!.id, userId: "outsider", role: "owner" });
});
