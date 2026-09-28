/**
 * `field.string().max(n)` manifest contract:
 *   - the limit serializes as `maxLength` (the Rust `ManifestField`
 *     reads that key), and only when set
 *   - it composes with the other modifiers in any order
 *   - it is refused on non-text fields and for non-positive limits
 */
import { expect, test } from "bun:test";
import { entitiesToManifest, field } from "./index";

test(".max(n) serializes as maxLength on string and richtext fields", () => {
  const [ent] = entitiesToManifest([
    {
      name: "Post",
      fields: {
        title: field.string().max(120),
        body: field.richtext().optional().max(20_000),
        status: field.enum(["draft", "live"]).max(5),
        slug: field.string(),
      },
    },
  ]);
  const byName = (n: string) => ent.fields.find((f) => f.name === n)!;
  expect(byName("title").maxLength).toBe(120);
  expect(byName("body").maxLength).toBe(20_000);
  expect(byName("body").optional).toBe(true);
  expect(byName("status").maxLength).toBe(5);
  // No limit, no key: unannotated manifests are unchanged.
  expect("maxLength" in byName("slug")).toBe(false);
});

test(".max(n) is refused on other field types and for bad limits", () => {
  expect(() => (field.int() as { max(n: number): unknown }).max(5)).toThrow(/field.string/);
  expect(() => field.json().max(5)).toThrow(/field.string/);
  expect(() => field.string().max(0)).toThrow(/positive integer/);
  expect(() => field.string().max(1.5)).toThrow(/positive integer/);
  expect(() => field.string().max(-1)).toThrow(/positive integer/);
  expect(() => field.string().max(1)).not.toThrow();
});
