// The ProseMirror helpers against the row-doc contract: the server creates
// a prosemirror field's container under the "row" map when the row is
// inserted (crates/crdt/src/prosemirror.rs); clients only bind to it.
import { describe, expect, test } from "bun:test";
import { LoroDoc, LoroMap } from "loro-crdt";
import { Schema } from "prosemirror-model";
import { schema as basic } from "prosemirror-schema-basic";
import {
  configureProseMirrorMarks,
  findProseMirrorContainer,
  readProseMirror,
  waitForProseMirrorContainer,
  writeProseMirror,
} from "./index";

// The basic schema plus a mark with an id, like a suggestion.
const schema = new Schema({
  nodes: basic.spec.nodes,
  marks: basic.spec.marks.addToEnd("suggestion", {
    attrs: { id: { default: null } },
    inclusive: false,
  }),
});

const doc = (...paragraphs: Array<Array<{ text: string; marks?: string[] }>>) =>
  schema.node(
    "doc",
    null,
    paragraphs.map((runs) =>
      schema.node(
        "paragraph",
        null,
        runs.map((r) =>
          schema.text(
            r.text,
            (r.marks ?? []).map((m) => (m === "suggestion" ? schema.mark("suggestion", { id: "s1" }) : schema.mark(m))),
          ),
        ),
      ),
    ),
  );

function peer(id: bigint): LoroDoc {
  const d = new LoroDoc();
  d.setPeerId(id);
  configureProseMirrorMarks(d, schema);
  return d;
}

/** A row doc as the server makes it on insert: an empty container under row.body. */
function serverRow(id: bigint): LoroDoc {
  const d = peer(id);
  d.getMap("row").set("title", "t");
  d.getMap("row").setContainer("body", new LoroMap());
  d.commit();
  return d;
}

describe("ProseMirror documents in a row", () => {
  test("the lookup never creates the container", () => {
    const d = peer(1n);
    expect(findProseMirrorContainer(d, "body")).toBeNull();
    expect(d.getMap("row").get("body")).toBeUndefined();
  });

  // The case a review reproduced: useLoroDoc hands the editor an empty doc
  // before the server's snapshot arrives. A container made then (cid:0@1)
  // loses the key to the server's (cid:0@2), and the editor's edits never
  // reach the row. Waiting binds to the server's container instead.
  test("an editor that loads before the snapshot binds to the server's container", async () => {
    const client = peer(1n);
    const bound = waitForProseMirrorContainer(client, "body");
    const server = serverRow(2n);
    client.import(server.export({ mode: "snapshot" }));
    const id = await bound;
    expect(id).toBe(findProseMirrorContainer(server, "body")!);
    expect(id).toContain("@2");

    // The editor's first write lands in the container the server reads.
    writeProseMirror(client, "body", schema, doc([{ text: "Typed " }, { text: "here", marks: ["strong"] }]));
    server.import(client.export({ mode: "update", from: server.oplogVersion() }));
    expect(readProseMirror(server, "body", schema)?.toJSON()).toEqual(
      doc([{ text: "Typed " }, { text: "here", marks: ["strong"] }]).toJSON(),
    );
  });

  test("waiting resolves at once when the container is there, and can be stopped", async () => {
    const d = serverRow(2n);
    expect(await waitForProseMirrorContainer(d, "body")).toBe(findProseMirrorContainer(d, "body")!);
    const controller = new AbortController();
    const pending = waitForProseMirrorContainer(peer(1n), "body", { signal: controller.signal });
    controller.abort(new Error("closed"));
    await expect(pending).rejects.toThrow("closed");
  });

  test("a write into a doc without the container throws instead of creating it", () => {
    const d = peer(1n);
    expect(() => writeProseMirror(d, "body", schema, doc([{ text: "x" }]))).toThrow("the server creates it");
    expect(d.getMap("row").get("body")).toBeUndefined();
  });

  test("the server's empty container reads as null and takes a document", () => {
    const d = serverRow(2n);
    expect(readProseMirror(d, "body", schema)).toBeNull();
    const source = doc([{ text: "Hello " }, { text: "world", marks: ["strong", "suggestion"] }], [{ text: "Second" }]);
    writeProseMirror(d, "body", schema, source);
    expect(readProseMirror(d, "body", schema)?.toJSON()).toEqual(source.toJSON());
  });

  test("concurrent edits from two peers converge", () => {
    const a = serverRow(1n);
    writeProseMirror(a, "body", schema, doc([{ text: "One" }], [{ text: "Two" }]));
    const b = peer(2n);
    b.import(a.export({ mode: "snapshot" }));
    writeProseMirror(a, "body", schema, doc([{ text: "One!" }], [{ text: "Two" }]));
    writeProseMirror(b, "body", schema, doc([{ text: "One" }], [{ text: "Two?" }]));
    a.import(b.export({ mode: "update" }));
    b.import(a.export({ mode: "update" }));
    const left = readProseMirror(a, "body", schema)?.textContent;
    expect(left).toBe(readProseMirror(b, "body", schema)?.textContent);
    expect(left).toBe("One!Two?");
  });

  test("a server write reaches an editor's copy", () => {
    const editor = serverRow(1n);
    writeProseMirror(editor, "body", schema, doc([{ text: "Draft" }]));
    const server = peer(2n);
    server.import(editor.export({ mode: "snapshot" }));
    const current = readProseMirror(server, "body", schema)!;
    writeProseMirror(server, "body", schema, current.copy(current.content.append(doc([{ text: "Added by the server" }]).content)));
    editor.import(server.export({ mode: "update", from: editor.oplogVersion() }));
    expect(readProseMirror(editor, "body", schema)?.textContent).toBe("DraftAdded by the server");
  });
});
