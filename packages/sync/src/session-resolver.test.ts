// SessionResolver — unit tests for the verdicts the engine consumes.
// Pins the null→X / X→Y / token-flip rules in one place so the engine
// doesn't have to re-test them through its full pull/reconcile loop.

import { describe, expect, test } from "bun:test";

import { SessionResolver, sessionSignature } from "./session-resolver";

const anon = { userId: null, tenantId: null, isAdmin: false, roles: [] };
const u1OrgA = { userId: "u1", tenantId: "org-a", isAdmin: false, roles: [] };
const u1OrgB = { userId: "u1", tenantId: "org-b", isAdmin: false, roles: [] };
const u1NoTenant = {
  userId: "u1",
  tenantId: null,
  isAdmin: false,
  roles: [],
};

describe("SessionResolver", () => {
  test("first observation seeds state without invalidating the replica", () => {
    const r = new SessionResolver();
    const v = r.observeSession(u1OrgA);
    expect(v.tenantChanged).toBe(false);
    expect(v.replicaInvalidated).toBe(false);
    expect(v.isFirstResolution).toBe(false);
    expect(v.identityChanged).toBe(true); // null → u1
    expect(r.resolved()).toEqual(u1OrgA);
  });

  test("null → X flip is first-resolution (pull but NO reset)", () => {
    const r = new SessionResolver();
    r.observeSession(u1NoTenant); // seed lastSeenTenant=null
    const v = r.observeSession(u1OrgA);
    expect(v.tenantChanged).toBe(true);
    expect(v.isFirstResolution).toBe(true);
    expect(v.replicaInvalidated).toBe(false);
  });

  test("X → Y flip invalidates replica AND requires pull", () => {
    const r = new SessionResolver();
    r.observeSession(u1OrgA);
    const v = r.observeSession(u1OrgB);
    expect(v.tenantChanged).toBe(true);
    expect(v.isFirstResolution).toBe(false);
    expect(v.replicaInvalidated).toBe(true);
  });

  test("X → null (sign out) invalidates replica", () => {
    const r = new SessionResolver();
    r.observeSession(u1OrgA);
    const v = r.observeSession(anon);
    expect(v.tenantChanged).toBe(true);
    expect(v.isFirstResolution).toBe(false);
    expect(v.replicaInvalidated).toBe(true);
  });

  test("identical session is a no-op", () => {
    const r = new SessionResolver();
    r.observeSession(u1OrgA);
    const v = r.observeSession({ ...u1OrgA });
    expect(v.identityChanged).toBe(false);
    expect(v.tenantChanged).toBe(false);
    expect(v.replicaInvalidated).toBe(false);
  });

  test("role change is identity-only (no tenant flip, no reset)", () => {
    const r = new SessionResolver();
    r.observeSession(u1OrgA);
    const v = r.observeSession({ ...u1OrgA, roles: ["editor"] });
    expect(v.identityChanged).toBe(true);
    expect(v.tenantChanged).toBe(false);
    expect(v.replicaInvalidated).toBe(false);
  });

  test("token flip detection", () => {
    const r = new SessionResolver();
    expect(r.observeToken("tok-a").tokenChanged).toBe(false); // first observation
    expect(r.observeToken("tok-a").tokenChanged).toBe(false);
    expect(r.observeToken("tok-b").tokenChanged).toBe(true);
    expect(r.observeToken("tok-b").tokenChanged).toBe(false);
    expect(r.observeToken(null).tokenChanged).toBe(true); // sign-out
  });

  test("signature changes when any field changes", () => {
    expect(sessionSignature(u1OrgA)).not.toBe(sessionSignature(u1OrgB));
    expect(sessionSignature(u1OrgA)).not.toBe(
      sessionSignature({ ...u1OrgA, roles: ["editor"] }),
    );
    // Role order doesn't affect the signature (sorted before joining).
    expect(
      sessionSignature({ ...u1OrgA, roles: ["a", "b"] }),
    ).toBe(sessionSignature({ ...u1OrgA, roles: ["b", "a"] }));
  });
});
