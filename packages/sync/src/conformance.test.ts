/**
 * Shared sync-engine conformance scenarios.
 *
 * Every JSON file under `packages/sync/conformance/` runs here against
 * the TS engine and, from the same files, against the Swift engine
 * (`packages/swift/Tests/PylonSyncTests/SyncConformanceTests.swift`).
 * A behavior fixed in one engine gets a scenario here so the other
 * engine cannot drift. See `conformance/README.md` for the step schema.
 */
import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { createTestEnv, type TestEnv } from "./test-harness";
import type { Row } from "./types";

type Step =
  | { op: "seed"; entity: string; row_id: string; kind: "insert" | "update" | "delete"; data?: Row }
  | { op: "pull" }
  | { op: "frame"; frame: Record<string, unknown> }
  | { op: "update"; entity: string; id: string; data: Row }
  | { op: "delete"; entity: string; id: string }
  | { op: "expectRow"; entity: string; id: string; present: boolean; fields?: Row }
  | { op: "expectCount"; entity: string; count: number }
  | { op: "expectCursor"; last_seq: number };

interface Scenario {
  name: string;
  steps: Step[];
}

const USER = "u1";
const dir = join(import.meta.dir, "..", "conformance");
const files = readdirSync(dir)
  .filter((f) => f.endsWith(".json"))
  .sort();

describe("sync conformance", () => {
  let env: TestEnv | null = null;

  afterEach(async () => {
    await env?.dispose();
    env = null;
  });

  for (const file of files) {
    const scenario = JSON.parse(readFileSync(join(dir, file), "utf8")) as Scenario;
    test(`${file}: ${scenario.name}`, async () => {
      env = createTestEnv({ reconnectDelay: 1 });
      env.signIn({ userId: USER });
      await env.start();
      await env.flush(30);

      for (const [index, step] of scenario.steps.entries()) {
        const where = `${file} step ${index + 1} (${step.op})`;
        switch (step.op) {
          case "seed": {
            if (step.kind === "insert") {
              env.server.insert(step.entity, { ...(step.data ?? {}), id: step.row_id } as Row);
            } else if (step.kind === "update") {
              env.server.update(step.entity, step.row_id, step.data ?? {});
            } else {
              env.server.delete(step.entity, step.row_id);
            }
            // The harness fans server writes out over the mock WS; a
            // scenario asserts through `pull`, so drain that delivery
            // first to keep the two runners on the same path.
            await env.flush(30);
            break;
          }
          case "pull":
            await env.engine.pull();
            await env.flush(30);
            break;
          case "frame":
            env.server.pushToUser(USER, step.frame);
            await env.flush(30);
            break;
          case "update":
            await env.engine.update(step.entity, step.id, step.data);
            await env.flush(30);
            break;
          case "delete":
            await env.engine.delete(step.entity, step.id);
            await env.flush(30);
            break;
          case "expectRow": {
            const row = env.engine.store.get(step.entity, step.id) as Row | null;
            if (!step.present) {
              expect(row, where).toBeNull();
              break;
            }
            expect(row, where).not.toBeNull();
            for (const [key, value] of Object.entries(step.fields ?? {})) {
              expect((row as Row)[key], `${where} field ${key}`).toEqual(value);
            }
            break;
          }
          case "expectCount":
            expect(env.engine.store.list(step.entity).length, where).toBe(step.count);
            break;
          case "expectCursor":
            expect(
              (env.engine as unknown as { cursor: { last_seq: number } }).cursor.last_seq,
              where,
            ).toBe(step.last_seq);
            break;
        }
      }
    });
  }
});
