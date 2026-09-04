// The OpenAPI document is a contract with machines: an LLM function-calling
// layer turns each operation into a callable tool, using the operationId as
// the name and the descriptions as the entire basis for choosing it. These
// tests assert the properties that make that work, and the one that makes it
// honest — that every documented path is a route this app actually serves.

import { describe, expect, test } from "bun:test";
import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { openApiDocument } from "./openapi";

const doc = openApiDocument() as any;
const operations = Object.entries<any>(doc.paths).flatMap(([path, item]) =>
  Object.entries<any>(item).map(([method, op]) => ({ path, method, op })),
);

describe("document", () => {
  test("is OpenAPI 3.1 with a server and contact details", () => {
    expect(doc.openapi).toBe("3.1.0");
    expect(doc.servers[0].url).toBe("https://www.pylonsync.com");
    expect(doc.info.contact.email).toBe("support@pylonsync.com");
  });

  test("serializes to JSON without cycles or undefined", () => {
    const json = JSON.stringify(doc);
    expect(json.length).toBeGreaterThan(1000);
    expect(JSON.parse(json)).toEqual(doc);
  });
});

describe("operations", () => {
  test("every operationId is unique and camelCase", () => {
    const ids = operations.map((o) => o.op.operationId);
    expect(ids.every(Boolean)).toBe(true);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(id).toMatch(/^[a-z][A-Za-z0-9]*$/);
  });

  test("every operation has a summary and a description a model can act on", () => {
    for (const { path, method, op } of operations) {
      expect(op.summary, `${method} ${path}`).toBeTruthy();
      expect(op.description?.length ?? 0, `${method} ${path} description`).toBeGreaterThan(40);
    }
  });

  test("every parameter is typed and described", () => {
    for (const { path, method, op } of operations) {
      for (const p of op.parameters ?? []) {
        expect(p.description, `${method} ${path} ?${p.name}`).toBeTruthy();
        expect(p.schema?.type, `${method} ${path} ?${p.name} schema`).toBeTruthy();
        expect(typeof p.required).toBe("boolean");
      }
    }
  });

  test("every response declares content, except the ones with no body", () => {
    const bodyless = new Set(["202", "405"]);
    for (const { path, method, op } of operations) {
      for (const [status, res] of Object.entries<any>(op.responses)) {
        expect(res.description, `${method} ${path} ${status}`).toBeTruthy();
        if (bodyless.has(status)) continue;
        const media = Object.values<any>(res.content ?? {});
        expect(media.length, `${method} ${path} ${status} content`).toBeGreaterThan(0);
        for (const m of media) expect(m.schema, `${method} ${path} ${status} schema`).toBeTruthy();
      }
    }
  });

  test("the documented GET paths are all implemented by a route.ts", () => {
    // The failure this catches: publishing a spec that describes an endpoint
    // nobody built. An agent trusts the document over the site.
    const appDir = join(import.meta.dir, "..", "..", "app");
    const routes = new Set<string>();
    const walk = (dir: string, segments: string[]) => {
      for (const entry of readdirSync(dir)) {
        const full = join(dir, entry);
        if (statSync(full).isDirectory()) {
          walk(full, [...segments, entry]);
        } else if (entry === "route.ts") {
          routes.add(`/${segments.join("/")}`);
        }
      }
    };
    walk(appDir, []);
    for (const { path } of operations) {
      expect(routes.has(path), `${path} has no route.ts`).toBe(true);
    }
  });
});
