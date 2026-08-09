// Unit tests for `ctx.requireMember` (the org membership / role gate). The
// motivating bug class: actions + mutations bypass entity read policies, so a
// function that trusts an attacker-supplied orgId is an IDOR unless it
// re-checks membership. These assert the gate rejects non-members + wrong
// roles, and reads the right entity/fields.
import { describe, expect, test } from "bun:test";
import { makeRequireMember } from "./member";

// A spy `read` that records its (entity, filter) and returns canned rows.
function spyRead(rows: any[]) {
  const calls: Array<{ entity: string; filter: Record<string, unknown> }> = [];
  const read = async (entity: string, filter: Record<string, unknown>) => {
    calls.push({ entity, filter });
    return rows;
  };
  return { read, calls };
}

async function codeOf(fn: () => Promise<unknown>): Promise<string | undefined> {
  try {
    await fn();
    return undefined;
  } catch (e: any) {
    return e?.code;
  }
}

describe("ctx.requireMember", () => {
  test("UNAUTHENTICATED when there is no signed-in user", async () => {
    const { read, calls } = spyRead([{ role: "owner" }]);
    const requireMember = makeRequireMember(null, read);
    expect(await codeOf(() => requireMember("org_1"))).toBe("UNAUTHENTICATED");
    // Must short-circuit BEFORE touching the database.
    expect(calls).toEqual([]);
  });

  test("MISSING_ORG when orgId is empty", async () => {
    const { read } = spyRead([{ role: "owner" }]);
    const requireMember = makeRequireMember("u1", read);
    expect(await codeOf(() => requireMember(""))).toBe("MISSING_ORG");
  });

  test("FORBIDDEN when the caller is not a member (no row)", async () => {
    const { read } = spyRead([]);
    const requireMember = makeRequireMember("u1", read);
    expect(await codeOf(() => requireMember("org_1"))).toBe("FORBIDDEN");
  });

  test("returns the membership row for any member when no role is required", async () => {
    const row = { id: "m1", role: "member", userId: "u1", orgId: "org_1" };
    const { read, calls } = spyRead([row]);
    const requireMember = makeRequireMember("u1", read);
    expect(await requireMember("org_1")).toEqual(row);
    // Reads the conventional OrgMember entity keyed on (orgId, userId).
    expect(calls).toEqual([
      { entity: "OrgMember", filter: { orgId: "org_1", userId: "u1" } },
    ]);
  });

  test("role gate: FORBIDDEN when the member's role isn't allowed", async () => {
    const { read } = spyRead([{ role: "member" }]);
    const requireMember = makeRequireMember("u1", read);
    expect(
      await codeOf(() => requireMember("org_1", { role: ["owner", "admin"] })),
    ).toBe("FORBIDDEN");
  });

  test("custom roles are exact-match and inherit no built-in role", async () => {
    const reviewer = { id: "m1", role: "reviewer" };
    const requireMember = makeRequireMember("u1", spyRead([reviewer]).read);

    expect(
      await codeOf(() =>
        requireMember("org_1", { role: ["owner", "admin", "member"] }),
      ),
    ).toBe("FORBIDDEN");
    expect(await requireMember("org_1", { role: ["reviewer"] })).toEqual(
      reviewer,
    );
  });

  test("role gate: passes when the member's role IS allowed (string or array)", async () => {
    const owner = { id: "m1", role: "owner" };
    expect(
      await makeRequireMember("u1", spyRead([owner]).read)("org_1", {
        role: "owner",
      }),
    ).toEqual(owner);
    expect(
      await makeRequireMember("u1", spyRead([owner]).read)("org_1", {
        role: ["owner", "admin"],
      }),
    ).toEqual(owner);
  });

  test("honors a custom membership entity + field names", async () => {
    const { read, calls } = spyRead([{ tier: "admin" }]);
    const requireMember = makeRequireMember("u1", read);
    await requireMember("team_9", {
      entity: "TeamMember",
      orgField: "teamId",
      userField: "memberId",
      roleField: "tier",
      role: "admin",
    });
    expect(calls).toEqual([
      { entity: "TeamMember", filter: { teamId: "team_9", memberId: "u1" } },
    ]);
  });
});
