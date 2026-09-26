import { describe, expect, test } from "bun:test";
import { demoEmail, isDemoRow } from "../lib/demo-team";

describe("demo teammates", () => {
  test("the first address is the person's own, then numbered aliases", () => {
    expect(demoEmail("maya.castillo@demo.invalid", 0)).toBe("maya.castillo@demo.invalid");
    expect(demoEmail("maya.castillo@demo.invalid", 1)).toBe("maya.castillo+2@demo.invalid");
    expect(demoEmail("maya.castillo@demo.invalid", 2)).toBe("maya.castillo+3@demo.invalid");
  });

  test("a registered account is never reused as a demo teammate", () => {
    // Someone who registered a demo address first must not be handed the
    // seeded records.
    expect(isDemoRow({ email: "x@demo.invalid", passwordHash: "argon2id$..." })).toBe(false);
    expect(isDemoRow({ email: "x@demo.invalid", emailVerified: "2026-07-01T00:00:00Z" })).toBe(false);
    expect(isDemoRow({ email: "x@demo.invalid", displayName: "Maya Castillo" })).toBe(true);
  });
});
