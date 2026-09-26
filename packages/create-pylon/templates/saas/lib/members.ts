import { personName } from "./names";

// Server-side membership lookups shared by the task functions. Functions
// bypass entity policies, so every lookup here is scoped to one workspace id
// the caller has already been checked against with `ctx.requireMember`.

type Reader = {
  unsafe: {
    get(entity: string, id: string): Promise<Record<string, unknown> | null>;
    query(entity: string, filter: Record<string, unknown>): Promise<Record<string, unknown>[]>;
  };
};

/** Every member of `orgId` with the name the UI shows for them. */
export async function workspaceMembers(
  db: Reader,
  orgId: string,
): Promise<{ userId: string; name: string; role: string }[]> {
  const rows = await db.unsafe.query("OrgMember", { orgId });
  const out: { userId: string; name: string; role: string }[] = [];
  for (const row of rows) {
    const userId = String(row.userId);
    const user = await db.unsafe.get("User", userId);
    out.push({
      userId,
      role: String(row.role ?? "member"),
      name: personName({
        displayName: (user?.displayName as string | undefined) ?? null,
        email: (user?.email as string | undefined) ?? null,
      }),
    });
  }
  return out;
}

/**
 * The display name of `userId` if they belong to `orgId`, else null. The
 * task functions call this before assigning, so a task can only point at a
 * member of its own workspace.
 */
export async function memberName(
  db: Reader,
  orgId: string,
  userId: string,
): Promise<string | null> {
  const rows = await db.unsafe.query("OrgMember", { orgId, userId });
  if (rows.length === 0) return null;
  const user = await db.unsafe.get("User", userId);
  return personName({
    displayName: (user?.displayName as string | undefined) ?? null,
    email: (user?.email as string | undefined) ?? null,
  });
}
