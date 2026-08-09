import { describe, expect, test } from "bun:test";
import { normalizeAuthClaims } from "./auth";

describe("function auth normalization", () => {
  test("preserves custom roles from the Rust wire envelope", () => {
    expect(
      normalizeAuthClaims({
        user_id: "u1",
        is_admin: false,
        tenant_id: "org_1",
        roles: ["reviewer"],
      }),
    ).toEqual({
      userId: "u1",
      isAdmin: false,
      tenantId: "org_1",
      roles: ["reviewer"],
    });
  });

  test("defaults missing roles to an empty array and drops non-strings", () => {
    expect(normalizeAuthClaims({}).roles).toEqual([]);
    expect(normalizeAuthClaims({ roles: ["billing", 42, null] }).roles).toEqual([
      "billing",
    ]);
  });
});
