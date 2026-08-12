// Function discovery (discoverFunctions) — the functions/ walk that fills
// buildManifest's queries/actions. The correctness bar is agreement with the
// runtime loader in packages/functions/src/runtime.ts: a manifest that lists
// something the router won't serve documents an endpoint that doesn't exist,
// and one that omits a live function under-reports the whole API.

import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { discoverFunctions } from "./index";

describe("discoverFunctions", () => {
  const tmpdirs: string[] = [];
  const prevCwd = process.cwd();
  afterEach(() => {
    process.chdir(prevCwd);
    for (const d of tmpdirs.splice(0)) {
      try {
        fs.rmSync(d, { recursive: true, force: true });
      } catch {
        /* best effort */
      }
    }
  });

  /** Materialize a functions/ dir (filename → module source) and chdir in. */
  function fns(files: Record<string, string>): void {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-fns-"));
    tmpdirs.push(root);
    for (const [rel, src] of Object.entries(files)) {
      const abs = path.join(root, "functions", rel);
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      fs.writeFileSync(abs, src);
    }
    process.chdir(root);
  }

  /**
   * A function module shaped like `@pylonsync/functions` builds one. Written
   * literally rather than imported so the SDK's test suite doesn't take a
   * dependency on the functions package.
   */
  const fn = (
    type: string,
    args: string = "{}",
    extra: string = "",
  ): string =>
    `export default { type: ${JSON.stringify(type)}, handler: () => null, args: ${args}${extra} };`;

  test("splits queries from writes", async () => {
    fns({
      "listEvents.ts": fn("query"),
      "saveEvent.ts": fn("mutation"),
      "sendInvite.ts": fn("action"),
    });
    const { queries, actions } = await discoverFunctions();
    expect(queries.map((q) => q.name)).toEqual(["listEvents"]);
    // Mutations and actions share the write bucket; fnType keeps them apart.
    expect(actions.map((a) => a.name).sort()).toEqual([
      "saveEvent",
      "sendInvite",
    ]);
    expect(actions.find((a) => a.name === "saveEvent")!.fnType).toBe("mutation");
    expect(actions.find((a) => a.name === "sendInvite")!.fnType).toBe("action");
  });

  test("excludes internal functions", async () => {
    // The router answers FN_NOT_FOUND for these, so listing one would
    // document an endpoint no external client can reach.
    fns({
      "publicOne.ts": fn("query"),
      "secretOne.ts": fn("query", "{}", ", internal: true"),
    });
    const { queries } = await discoverFunctions();
    expect(queries.map((q) => q.name)).toEqual(["publicOne"]);
  });

  test("maps validators onto manifest field types", async () => {
    fns({
      "saveTrack.ts": fn(
        "mutation",
        `{
          title: { type: "string" },
          seats: { type: "int" },
          price: { type: "float" },
          live: { type: "bool" },
          startsAt: { type: "datetime" },
          eventId: { type: "id", table: "Event" }
        }`,
      ),
    });
    const { actions } = await discoverFunctions();
    const byName = Object.fromEntries(
      actions[0]!.input!.map((f) => [f.name, f.type]),
    );
    expect(byName).toEqual({
      title: "string",
      seats: "int",
      price: "float",
      live: "bool",
      startsAt: "datetime",
      eventId: "id(Event)",
    });
  });

  test("reads optional off the validator and keeps the id table", async () => {
    // v.optional(v.id("Event")) spreads the inner validator and adds
    // optional:true, so `table` survives the wrapper.
    fns({
      "attach.ts": fn(
        "mutation",
        `{
          eventId: { type: "id", table: "Event", optional: true },
          note: { type: "string" }
        }`,
      ),
    });
    const { actions } = await discoverFunctions();
    const input = actions[0]!.input!;
    const eventId = input.find((f) => f.name === "eventId")!;
    expect(eventId.optional).toBe(true);
    expect(eventId.type).toBe("id(Event)");
    expect(input.find((f) => f.name === "note")!.optional).toBe(false);
  });

  test("unmapped validators collapse to json", async () => {
    // FieldType has no array or union variant. Documented lossiness —
    // widening FieldType is what would preserve Validator.items.
    fns({
      "bulk.ts": fn(
        "action",
        `{
          tags: { type: "array", items: { type: "string" } },
          payload: { type: "any" },
          blob: { type: "json" }
        }`,
      ),
    });
    const { actions } = await discoverFunctions();
    for (const field of actions[0]!.input!) {
      expect(field.type).toBe("json");
    }
  });

  test("carries the declared auth gate", async () => {
    fns({
      "publicFeed.ts": fn("query", "{}", `, auth: "public"`),
      "myProfile.ts": fn("query"),
    });
    const { queries } = await discoverFunctions();
    expect(queries.find((q) => q.name === "publicFeed")!.auth).toBe("public");
    // Undeclared stays undefined rather than being invented — the
    // runtime's default is "user" and consumers apply it.
    expect(queries.find((q) => q.name === "myProfile")!.auth).toBeUndefined();
  });

  test("skips modules whose default export is not a function definition", async () => {
    fns({
      "real.ts": fn("query"),
      "config.ts": "export default { apiKey: 'x' };",
      "noDefault.ts": "export const helper = () => null;",
      // `type` present but no handler — the runtime's shape check
      // rejects this too.
      "halfBaked.ts": "export default { type: 'query' };",
    });
    const { queries, actions } = await discoverFunctions();
    expect(queries.map((q) => q.name)).toEqual(["real"]);
    expect(actions).toEqual([]);
  });

  test("a module that throws on import is skipped, not fatal", async () => {
    fns({
      "good.ts": fn("query"),
      "explodes.ts": "throw new Error('boom');",
    });
    const { queries } = await discoverFunctions();
    expect(queries.map((q) => q.name)).toEqual(["good"]);
  });

  test("no functions directory yields empty, not an error", async () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-fns-"));
    tmpdirs.push(root);
    process.chdir(root);
    expect(await discoverFunctions()).toEqual({ queries: [], actions: [] });
  });

  test("output is sorted so two machines agree", async () => {
    fns({
      "zebra.ts": fn("query"),
      "alpha.ts": fn("query"),
      "middle.ts": fn("query"),
    });
    const { queries } = await discoverFunctions();
    expect(queries.map((q) => q.name)).toEqual(["alpha", "middle", "zebra"]);
  });

  test("ignores non-module files the runtime would also ignore", async () => {
    fns({
      "real.ts": fn("query"),
      "notes.md": "# not a function",
      "data.json": "{}",
    });
    const { queries } = await discoverFunctions();
    expect(queries.map((q) => q.name)).toEqual(["real"]);
  });
});
