import { describe, expect, test } from "bun:test";
import { buildManifest, entity, field } from "./index";

/**
 * The shape callers actually hit.
 *
 * `EntityDefinition.sync` and the `entity(name, fields, options)` options
 * parameter declare the same field twice, and they drifted the moment
 * `SyncScope` was added: the interface accepted a scope, the function did not,
 * so `sync: { limit: 2000 }` was a type error in every real app while the
 * feature looked shipped. These assert through `entity()` — the public door —
 * so the two can't diverge again unnoticed.
 */
describe("entity() sync option", () => {
  const fields = { orgId: field.string(), views: field.int() };

  const manifestFor = (e: ReturnType<typeof entity>) =>
    buildManifest({
      name: "t",
      version: "1",
      entities: [e],
      policies: [],
      queries: [],
      actions: [],
      routes: [],
    }) as unknown as {
      entities: Array<{
        name: string;
        sync?: boolean;
        sync_scope?: string;
        sync_limit?: number;
      }>;
    };

  test("accepts a scope object, not just a boolean", () => {
    const e = entity("Snap", fields, {
      sync: { where: "data.orgId == auth.tenantId", limit: 2000 },
    });
    expect(e.sync).toEqual({
      where: "data.orgId == auth.tenantId",
      limit: 2000,
    });
  });

  test("a scope reaches the manifest as flat sibling keys", () => {
    // Flattened so `sync` stays a bool on the wire — an older binary reading a
    // newer manifest replicates everything instead of failing to parse.
    const snap = manifestFor(
      entity("Snap", fields, {
        sync: { where: "data.orgId == auth.tenantId", limit: 2000 },
      }),
    ).entities[0];
    expect(snap.sync).toBeUndefined(); // still the default: true
    expect(snap.sync_scope).toBe("data.orgId == auth.tenantId");
    expect(snap.sync_limit).toBe(2000);
  });

  test("a limit with no predicate emits no scope expression", () => {
    const snap = manifestFor(entity("Snap", fields, { sync: { limit: 500 } }))
      .entities[0];
    expect(snap.sync_scope).toBeUndefined();
    expect(snap.sync_limit).toBe(500);
  });

  test("sync: false still opts out entirely", () => {
    expect(manifestFor(entity("Snap", fields, { sync: false })).entities[0].sync)
      .toBe(false);
  });
});
