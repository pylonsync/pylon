// LiveDoc and editProseMirror against a fake ctx.crdt whose "server" is a
// LoroDoc, the way the host's store merges and serves a row's document.
import { describe, expect, test } from "bun:test";
import { LoroDoc, LoroList, LoroMap, VersionVector } from "loro-crdt";
import { Schema } from "prosemirror-model";
import { schema as basic } from "prosemirror-schema-basic";
import { editProseMirror, readProseMirror, writeProseMirror, configureProseMirrorMarks } from "./prosemirror/index";
import { openLiveDoc, type CrdtOps, type LiveDocContext } from "./server";

function fakeHost(server: LoroDoc | null) {
  const applied: Uint8Array[] = [];
  const calls: string[] = [];
  const ops = (label: string): CrdtOps => ({
    async snapshot() {
      calls.push(`${label}snapshot`);
      return server ? server.export({ mode: "snapshot" }) : null;
    },
    async since(_e, _i, version) {
      calls.push(`${label}since`);
      return server ? server.export({ mode: "update", from: VersionVector.decode(version) }) : null;
    },
    async apply(_e, _i, update) {
      calls.push(`${label}apply`);
      applied.push(update);
      server!.import(update);
    },
  });
  const ctx: LiveDocContext = { crdt: { ...ops(""), unsafe: ops("unsafe:") } };
  return { ctx, applied, calls };
}

function rowDoc(): LoroDoc {
  const d = new LoroDoc();
  d.getMap("row").set("title", "start");
  d.commit();
  return d;
}

describe("LiveDoc", () => {
  test("an edit reaches the server", async () => {
    const server = rowDoc();
    const { ctx } = fakeHost(server);
    const live = await openLiveDoc(ctx, "Doc", "d1");
    await live.edit((doc) => doc.getMap("row").set("status", "done"));
    expect(server.getMap("row").get("status")).toBe("done");
  });

  test("edit pulls in what others wrote after the doc was opened", async () => {
    const server = rowDoc();
    const text = server.getMap("row").setContainer("body", new (await import("loro-crdt")).LoroText());
    server.commit();
    const { ctx } = fakeHost(server);
    const live = await openLiveDoc(ctx, "Doc", "d1");
    // Someone types at the server's copy meanwhile.
    text.insert(0, "person ");
    server.commit();
    await live.edit((doc) => {
      const t = doc.getMap("row").get("body") as { insert(i: number, s: string): void; length: number };
      t.insert(t.length, "function");
    });
    expect(text.toString()).toBe("person function");
  });

  test("commit sends only this function's ops, once", async () => {
    const server = rowDoc();
    const big = new LoroDoc();
    big.import(server.export({ mode: "snapshot" }));
    big.getMap("row").set("blob", "x".repeat(5000));
    big.commit();
    server.import(big.export({ mode: "update" }));
    const { ctx, applied } = fakeHost(server);
    const live = await openLiveDoc(ctx, "Doc", "d1");
    live.doc.getMap("row").set("n", 1);
    expect(await live.commit()).toBe(true);
    expect(await live.commit()).toBe(false);
    expect(applied.length).toBe(1);
    expect(applied[0].length).toBeLessThan(200);
  });

  test("a row without a document is NOT_FOUND", async () => {
    const { ctx } = fakeHost(null);
    await expect(openLiveDoc(ctx, "Doc", "nope")).rejects.toMatchObject({ code: "NOT_FOUND" });
  });

  test("unsafe goes through ctx.crdt.unsafe", async () => {
    const server = rowDoc();
    const { ctx, calls } = fakeHost(server);
    const live = await openLiveDoc(ctx, "Doc", "d1", { unsafe: true });
    await live.edit((doc) => doc.getMap("row").set("k", 1));
    expect(calls).toEqual(["unsafe:snapshot", "unsafe:since", "unsafe:apply"]);
  });
});

describe("editProseMirror", () => {
  const schema = new Schema({ nodes: basic.spec.nodes, marks: basic.spec.marks });
  const para = (s: string) => schema.node("paragraph", null, s ? [schema.text(s)] : []);

  function serverWithBody(): LoroDoc {
    const server = new LoroDoc();
    const body = server.getMap("row").setContainer("body", new LoroMap());
    body.setContainer("attributes", new LoroMap());
    body.setContainer("children", new LoroList());
    server.commit();
    return server;
  }

  test("a function's edit and a person's edit at the same time both land", async () => {
    const server = serverWithBody();
    configureProseMirrorMarks(server, schema);
    writeProseMirror(server, "body", schema, schema.node("doc", null, [para("Hello")]));
    const { ctx } = fakeHost(server);
    const live = await openLiveDoc(ctx, "Doc", "d1");
    // A person adds a paragraph after the function opened the doc.
    const now = readProseMirror(server, "body", schema)!;
    writeProseMirror(server, "body", schema, now.copy(now.content.append(schema.node("doc", null, [para("from a person")]).content)));
    const written = await editProseMirror(live, "body", schema, (state) => state.tr.insertText("Summary: ", 1));
    expect(written).not.toBeNull();
    const result: string[] = [];
    readProseMirror(server, "body", schema)!.forEach((n) => result.push(n.textContent));
    expect(result).toEqual(["Summary: Hello", "from a person"]);
  });

  test("an empty document starts from the schema's empty document", async () => {
    const server = serverWithBody();
    const { ctx } = fakeHost(server);
    const live = await openLiveDoc(ctx, "Doc", "d1");
    await editProseMirror(live, "body", schema, (state) => state.tr.insertText("first", 1));
    expect(readProseMirror(server, "body", schema)!.textContent).toBe("first");
  });

  test("returning nothing changes nothing", async () => {
    const server = serverWithBody();
    const { ctx, applied } = fakeHost(server);
    const live = await openLiveDoc(ctx, "Doc", "d1");
    expect(await editProseMirror(live, "body", schema, () => undefined)).toBeNull();
    expect(applied.length).toBe(0);
  });
});
