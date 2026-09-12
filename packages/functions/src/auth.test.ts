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
      isGuest: false,
    });
  });

  test("a guest session is marked as one", () => {
    // The guest id is a real string, so a handler checking only
    // `userId != null` cannot tell it from an account without this flag.
    expect(
      normalizeAuthClaims({ user_id: "guest_a1b2", is_guest: true }),
    ).toEqual({
      userId: "guest_a1b2",
      isAdmin: false,
      tenantId: null,
      roles: [],
      isGuest: true,
    });
    expect(normalizeAuthClaims({ userId: "guest_a1b2", isGuest: true }).isGuest).toBe(
      true,
    );
  });

  test("a payload from before the field existed reads as not-a-guest", () => {
    expect(normalizeAuthClaims({ user_id: "u1" }).isGuest).toBe(false);
  });

  test("defaults missing roles to an empty array and drops non-strings", () => {
    expect(normalizeAuthClaims({}).roles).toEqual([]);
    expect(normalizeAuthClaims({ roles: ["billing", 42, null] }).roles).toEqual([
      "billing",
    ]);
  });
});
