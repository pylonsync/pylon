/**
 * `field.vector(dims)` manifest contract:
 *   - the wire type string is `vector(<dims>)`
 *   - vector fields serialize with `serverOnly: true` unconditionally
 *     (multi-KB embeddings must never enter sync or HTTP reads)
 *   - dims outside 1..8192 throw at schema-definition time
 */
import { expect, test } from "bun:test";
import { entitiesToManifest, field } from "./index";

test("field.vector serializes as vector(dims) with forced serverOnly", () => {
  const [ent] = entitiesToManifest([
    {
      name: "Doc",
      fields: {
        title: field.string(),
        embedding: field.vector(1536).optional(),
      },
    },
  ]);
  const emb = ent.fields.find((f) => f.name === "embedding")!;
  expect(emb.type).toBe("vector(1536)");
  expect(emb.optional).toBe(true);
  expect(emb.serverOnly).toBe(true);
  // Non-vector fields don't inherit the forcing.
  const title = ent.fields.find((f) => f.name === "title")!;
  expect(title.serverOnly).toBeUndefined();
});

test("field.vector rejects out-of-range dims", () => {
  expect(() => field.vector(0)).toThrow();
  expect(() => field.vector(1.5)).toThrow();
  expect(() => field.vector(8193)).toThrow();
  expect(() => field.vector(8192)).not.toThrow();
  expect(() => field.vector(1)).not.toThrow();
});
