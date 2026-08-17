/**
 * `hasAgents` manifest injection: buildManifest appends the
 * AgentRun/AgentMessage entities + owner-scoping policies when any
 * `agent()` was discovered — as ordinary synced entities, so every
 * downstream surface (sync, codegen, migrate) applies unchanged.
 */
import { expect, test } from "bun:test";
import { buildManifest, entity, field } from "./index";

const base = {
  name: "t",
  version: "0",
  entities: [entity("Doc", { title: field.string() })],
  routes: [],
};

test("hasAgents injects entities, indexes, sync scopes, and policies", () => {
  const m = buildManifest({ ...base, hasAgents: true });

  const run = m.entities.find((e) => e.name === "AgentRun")!;
  expect(run).toBeDefined();
  const runFields = Object.fromEntries(run.fields.map((f) => [f.name, f.type]));
  expect(runFields.agent).toBe("string");
  expect(runFields.status).toBe("string");
  expect(runFields.streamId).toBe("string");
  expect(runFields.createdAt).toBe("datetime");
  expect(run.sync_scope).toBe("auth.userId != null && data.userId == auth.userId");

  const msg = m.entities.find((e) => e.name === "AgentMessage")!;
  expect(msg).toBeDefined();
  const msgFields = Object.fromEntries(msg.fields.map((f) => [f.name, f.type]));
  expect(msgFields.runId).toBe("id(AgentRun)");
  expect(msgFields.content).toBe("json");
  expect(msgFields.seq).toBe("int");
  expect(msg.indexes.some((i) => i.name === "by_run")).toBe(true);
  expect(msg.sync_scope).toBe("auth.userId != null && data.userId == auth.userId");

  const runPolicy = m.policies.find((p) => p.entity === "AgentRun")!;
  expect(runPolicy.allowRead).toBe("auth.userId != null && data.userId == auth.userId");
  expect(runPolicy.allowInsert).toBe("false");
  expect(runPolicy.allowUpdate).toBe("false");
  expect(runPolicy.allowDelete).toBe("false");
  expect(m.policies.some((p) => p.entity === "AgentMessage")).toBe(true);
});

test("without hasAgents nothing is injected", () => {
  const m = buildManifest(base);
  expect(m.entities.some((e) => e.name === "AgentRun")).toBe(false);
  expect(m.policies.some((p) => p.entity === "AgentRun")).toBe(false);
});

test("app-declared AgentRun and policies win over injection", () => {
  const custom = entity("AgentRun", { custom: field.string() });
  const m = buildManifest({
    ...base,
    entities: [...base.entities, custom],
    policies: [
      { name: "mine", entity: "AgentRun", allowRead: "auth.userId != null" },
    ],
    hasAgents: true,
  });
  const runs = m.entities.filter((e) => e.name === "AgentRun");
  expect(runs).toHaveLength(1);
  expect(runs[0].fields.map((f) => f.name)).toEqual(["custom"]);
  const policies = m.policies.filter((p) => p.entity === "AgentRun");
  expect(policies).toHaveLength(1);
  expect(policies[0].name).toBe("mine");
  // AgentMessage still injected (not declared by the app).
  expect(m.entities.some((e) => e.name === "AgentMessage")).toBe(true);
});

test("hasAgents is inferred from actions carrying isAgent", () => {
  const m = buildManifest({
    ...base,
    actions: [{ name: "helper", isAgent: true, fnType: "action" }],
  });
  expect(m.entities.some((e) => e.name === "AgentRun")).toBe(true);
  // Explicit false wins over inference.
  const off = buildManifest({
    ...base,
    actions: [{ name: "helper", isAgent: true, fnType: "action" }],
    hasAgents: false,
  });
  expect(off.entities.some((e) => e.name === "AgentRun")).toBe(false);
});
