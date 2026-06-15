// `ctx.requireMember` — the org membership / role authorization gate.
//
// Pure + dependency-injected so it's unit-testable and works across all three
// ctx types: query/mutation read membership via `ctx.db.query`, actions (which
// have no `ctx.db`) via `ctx.runQuery` to the built-in `__pylonMemberLookup`.
// runtime.ts wires the appropriate `read` per ctx; member.test.ts drives this
// directly.

import type { MemberRow, RequireMember } from "./types";

/**
 * Build `ctx.requireMember(orgId, opts)` bound to a membership `read`. Errors
 * carry `.code` (UNAUTHENTICATED / MISSING_ORG / FORBIDDEN) the same way
 * `ctx.error` does — self-contained so it also works on the query ctx, which
 * has no `ctx.error`.
 */
export function makeRequireMember(
  userId: string | null | undefined,
  read: (entity: string, filter: Record<string, unknown>) => Promise<any[]>,
): RequireMember {
  const fail = (code: string, message: string): never => {
    const e = new Error(message);
    (e as any).code = code;
    throw e;
  };
  return async (orgId, opts) => {
    if (!userId) fail("UNAUTHENTICATED", "log in first");
    if (!orgId) fail("MISSING_ORG", "orgId is required");
    const entity = opts?.entity ?? "OrgMember";
    const orgField = opts?.orgField ?? "orgId";
    const userField = opts?.userField ?? "userId";
    const roleField = opts?.roleField ?? "role";
    const rows = await read(entity, {
      [orgField]: orgId,
      [userField]: userId as string,
    });
    const row = rows && rows[0];
    if (!row) fail("FORBIDDEN", "you are not a member of this organization");
    if (opts?.role !== undefined) {
      const allowed = Array.isArray(opts.role) ? opts.role : [opts.role];
      if (!allowed.includes((row as any)[roleField])) {
        fail("FORBIDDEN", `requires one of: ${allowed.join(", ")}`);
      }
    }
    return row as MemberRow;
  };
}
