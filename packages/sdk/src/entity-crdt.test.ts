import { describe, expect, test } from "bun:test";
import { entitiesToManifest, entity, field } from "./index";

describe("entity crdt option", () => {
  test("crdt: false reaches the manifest", () => {
    const [m] = entitiesToManifest([entity("Rollup", { n: field.int() }, { crdt: false })]);
    expect(m.crdt).toBe(false);
  });

  test("the default is omitted, so the runtime default (true) applies", () => {
    const [a, b] = entitiesToManifest([
      entity("Note", { title: field.string() }),
      entity("Doc", { title: field.string() }, { crdt: true }),
    ]);
    expect("crdt" in a).toBe(false);
    expect("crdt" in b).toBe(false);
  });
});
