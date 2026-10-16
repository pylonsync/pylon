// The ProseMirror helpers against the row-doc contract: the server keeps a
// prosemirror field's tree under the "row" map (crates/crdt/src/prosemirror.rs).
import { describe, expect, test } from "bun:test";
import { LoroDoc } from "loro-crdt";
import { Schema } from "prosemirror-model";
import { schema as basic } from "prosemirror-schema-basic";
import {
  configureProseMirrorMarks,
  proseMirrorContainerId,
  readProseMirror,
  writeProseMirror,
} from "./prosemirror";

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

describe("ProseMirror documents in a row", () => {
  test("the container lives under the row map and is created once", () => {
    const d = peer(1n);
    const id = proseMirrorContainerId(d, "body");
    expect(proseMirrorContainerId(d, "body")).toBe(id);
    expect((d.getMap("row").get("body") as { id: string }).id).toBe(id);
  });

  test("a document round-trips, marks and their attrs included", () => {
    const d = peer(1n);
    const source = doc([{ text: "Hello " }, { text: "world", marks: ["strong", "suggestion"] }], [{ text: "Second" }]);
    writeProseMirror(d, "body", schema, source);
    expect(readProseMirror(d, "body", schema)?.toJSON()).toEqual(source.toJSON());
  });

  test("no document reads as null", () => {
    expect(readProseMirror(peer(1n), "body", schema)).toBeNull();
  });

  test("concurrent edits from two peers converge", () => {
    const a = peer(1n);
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
    const editor = peer(1n);
    writeProseMirror(editor, "body", schema, doc([{ text: "Draft" }]));
    const server = peer(2n);
    server.import(editor.export({ mode: "snapshot" }));
    const current = readProseMirror(server, "body", schema)!;
    writeProseMirror(server, "body", schema, current.copy(current.content.append(doc([{ text: "Added by the server" }]).content)));
    editor.import(server.export({ mode: "update", from: editor.oplogVersion() }));
    expect(readProseMirror(editor, "body", schema)?.textContent).toBe("DraftAdded by the server");
  });
});
