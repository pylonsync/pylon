// ProseMirrorSyncPlugin with two real editors (happy-dom) on two LoroDocs.
// `sync()` stands in for the network: it moves each doc's new ops to the
// other, the way the outbox and the server broadcast do.
import { GlobalRegistrator } from "@happy-dom/global-registrator";
if (!globalThis.document) GlobalRegistrator.register();

import { afterEach, describe, expect, test } from "bun:test";
import { LoroDoc, LoroList, LoroMap, type ContainerID } from "loro-crdt";
import { undo } from "loro-prosemirror";
import { type Node, Schema } from "prosemirror-model";
import { schema as basic } from "prosemirror-schema-basic";
import { EditorState, NodeSelection, TextSelection } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { configureProseMirrorMarks, findProseMirrorContainer, writeProseMirror } from "./index";
import { ProseMirrorSyncPlugin, ProseMirrorUndoPlugin } from "./sync";

const defaultSchema = new Schema({ nodes: basic.spec.nodes, marks: basic.spec.marks });
const tick = () => new Promise((r) => setTimeout(r, 0));

const views: EditorView[] = [];
afterEach(() => {
  for (const v of views.splice(0)) v.destroy();
});

/**
 * A server doc with the field's container, and two editors synced to it:
 * on two client docs, or with `shared` on one (two editors in one tab).
 */
function setup({ schema = defaultSchema, shared = false }: { schema?: Schema; shared?: boolean } = {}) {
  // The empty document the server makes on insert (crates/crdt prosemirror.rs).
  const server = new LoroDoc();
  const body = server.getMap("row").setContainer("body", new LoroMap());
  body.setContainer("attributes", new LoroMap());
  body.setContainer("children", new LoroList());
  server.commit();
  const docs = [new LoroDoc(), new LoroDoc()];
  for (const d of docs) {
    d.import(server.export({ mode: "snapshot" }));
    configureProseMirrorMarks(d, schema);
  }
  const containerId = findProseMirrorContainer(docs[0], "body") as ContainerID;
  const make = (doc: LoroDoc) => {
    const view = new EditorView(document.createElement("div"), {
      state: EditorState.create({
        schema,
        plugins: [ProseMirrorSyncPlugin({ doc, containerId }), ProseMirrorUndoPlugin({ doc })],
      }),
    });
    views.push(view);
    return view;
  };
  const [a, b] = [make(docs[0]), make(shared ? docs[0] : docs[1])];
  const sync = () => {
    for (const from of docs) {
      const bytes = from.export({ mode: "update", from: server.oplogVersion() });
      server.import(bytes);
    }
    const all = server.export({ mode: "snapshot" });
    for (const to of docs) to.import(all);
  };
  return { a, b, docs, server, sync };
}

const text = (v: EditorView) => v.state.doc.textBetween(0, v.state.doc.content.size, "\n");
const type = (v: EditorView, s: string) => {
  for (const ch of s) v.dispatch(v.state.tr.insertText(ch));
};
const caretAt = (v: EditorView, pos: number) =>
  v.dispatch(v.state.tr.setSelection(TextSelection.create(v.state.doc, pos)));

describe("ProseMirrorSyncPlugin", () => {
  test("typing in one editor reaches the other", async () => {
    const { a, b, sync } = setup();
    await tick();
    type(a, "Hello");
    sync();
    expect(text(b)).toBe("Hello");
  });

  test("a remote edit before the caret keeps the caret on its words", async () => {
    const { a, b, sync } = setup();
    await tick();
    type(a, "world");
    sync();
    caretAt(b, 6); // after "world"
    caretAt(a, 1);
    type(a, "hello ");
    sync();
    expect(text(b)).toBe("hello world");
    expect(b.state.selection.head).toBe(12);
  });

  test("typing right after a remote change goes at the caret, not the end", async () => {
    const { a, b, sync } = setup();
    await tick();
    type(a, "one three");
    sync();
    caretAt(b, 4); // after "one"
    caretAt(a, 10);
    type(a, " four");
    sync();
    type(b, " two"); // no timer in between
    sync();
    expect(text(a)).toBe("one two three four");
    expect(text(b)).toBe("one two three four");
  });

  test("two people typing at the same spot keep their words whole", async () => {
    const { a, b, sync } = setup();
    await tick();
    type(a, "Hi");
    sync();
    caretAt(a, 3);
    caretAt(b, 3);
    const left = " [from a]";
    const right = " [from b]";
    for (let i = 0; i < left.length; i++) {
      type(a, left[i]);
      type(b, right[i]);
      sync();
    }
    expect(text(a)).toBe(text(b));
    expect([`Hi${left}${right}`, `Hi${right}${left}`]).toContain(text(a));
  });

  test("a paragraph split elsewhere does not move the caret", async () => {
    const { a, b, sync } = setup();
    await tick();
    type(a, "first second");
    sync();
    caretAt(b, 13); // end of "second"
    caretAt(a, 6); // after "first"
    a.dispatch(a.state.tr.split(6));
    sync();
    type(b, "!");
    sync();
    expect(text(a)).toBe("first\n second!");
    expect(text(b)).toBe("first\n second!");
  });

  test("undo takes back a burst of typing as one step", async () => {
    const { a, sync } = setup();
    await tick();
    type(a, "kept");
    sync();
    await new Promise((r) => setTimeout(r, 600));
    type(a, " typed together");
    undo(a.state, a.dispatch);
    expect(text(a)).toBe("kept");
  });

  // Loro does not merge an undo step across a remote change in the same
  // container (loro-internal undo.rs, push_with_merge), so while two people
  // type in one paragraph each undo takes back a smaller step. Each step is
  // still only this person's.
  test("undo while someone else types removes only this person's typing", async () => {
    const { a, b, sync } = setup();
    await tick();
    type(a, "x");
    sync();
    await new Promise((r) => setTimeout(r, 600));
    caretAt(a, 2);
    caretAt(b, 1);
    for (const [ca, cb] of [["1", "a"], ["2", "b"], ["3", "c"], ["4", "d"]]) {
      type(a, ca);
      type(b, cb);
      sync();
    }
    undo(b.state, b.dispatch);
    sync();
    expect(text(a)).toContain("1234");
    expect(text(a)).not.toContain("d");
    for (let i = 0; i < 4; i++) undo(b.state, b.dispatch);
    sync();
    expect(text(a)).toBe("x1234");
    expect(text(b)).toBe("x1234");
  });

  test("undo removes only this editor's edit", async () => {
    const { a, b, sync } = setup();
    await tick();
    type(a, "base");
    sync();
    caretAt(b, 5);
    type(b, " by b");
    sync();
    caretAt(a, 1);
    type(a, "A:");
    sync();
    undo(b.state, b.dispatch);
    sync();
    expect(text(a)).toBe("A:base");
    expect(text(b)).toBe("A:base");
  });
});

// Regressions from the Codex review of 2026-10-17.
describe("ProseMirrorSyncPlugin review fixes", () => {
  test("the first edit in a new document can be undone", async () => {
    const { a, b, sync } = setup();
    await tick();
    type(a, "first words");
    sync();
    expect(text(b)).toBe("first words");
    await new Promise((r) => setTimeout(r, 600));
    type(a, " more");
    undo(a.state, a.dispatch);
    undo(a.state, a.dispatch);
    sync();
    expect(text(a)).toBe("");
    expect(text(b)).toBe("");
  });

  test("two editors on one doc see each other's edits", async () => {
    const { a, b, server, sync } = setup({ shared: true });
    await tick();
    type(a, "base");
    expect(text(b)).toBe("base");
    caretAt(b, 5);
    type(b, "B");
    expect(text(a)).toBe("baseB");
    sync();
    expect(JSON.stringify(server.getMap("row").get("body"))).toBeDefined();
    expect(text(a)).toBe("baseB");
  });

  test("the top node's attributes load and are not overwritten", async () => {
    const langSchema = new Schema({
      nodes: defaultSchema.spec.nodes.update("doc", { ...defaultSchema.spec.nodes.get("doc")!, attrs: { lang: { default: "en" } } }),
      marks: defaultSchema.spec.marks,
    });
    const { a, b, docs, sync } = setup({ schema: langSchema });
    await tick();
    a.dispatch(a.state.tr.setDocAttribute("lang", "fr"));
    type(a, "bonjour");
    sync();
    expect(b.state.doc.attrs.lang).toBe("fr");
    type(b, "!");
    sync();
    expect(a.state.doc.attrs.lang).toBe("fr");
    const attrs = (docs[0].getMap("row").get("body") as LoroMap).get("attributes") as LoroMap;
    expect(attrs.get("lang")).toBe("fr");
  });

  test("clearing the document does not add a paragraph per editor", async () => {
    const { a, b, docs, server, sync } = setup();
    await tick();
    type(a, "text");
    sync();
    const children = (server.getMap("row").get("body") as LoroMap).get("children") as LoroList;
    children.delete(0, children.length);
    server.commit();
    for (const d of docs) d.import(server.export({ mode: "snapshot" }));
    sync();
    expect(children.length).toBe(0);
    expect(text(a)).toBe("");
    expect(text(b)).toBe("");
  });

  test("a schema whose top node is not doc can edit a new document", async () => {
    const custom = new Schema({
      topNode: "document",
      nodes: defaultSchema.spec.nodes.remove("doc").addToStart("document", { content: "block+" }),
      marks: defaultSchema.spec.marks,
    });
    const { a, b, sync } = setup({ schema: custom });
    await tick();
    type(a, "hello");
    sync();
    expect(text(b)).toBe("hello");
    expect(b.state.doc.type.name).toBe("document");
  });

  test("a caret after an emoji stays after it", async () => {
    const { a, b, sync } = setup();
    await tick();
    type(a, "x😀");
    sync();
    caretAt(b, 4); // after the emoji (1 + 1 + 2)
    caretAt(a, 1);
    type(a, "AB");
    sync();
    expect(text(b)).toBe("ABx😀");
    expect(b.state.selection.head).toBe(6);
  });

  test("a caret after an inline node with content stays put", async () => {
    const nodes = defaultSchema.spec.nodes.addToEnd("chip", {
      group: "inline",
      inline: true,
      content: "text*",
      toDOM: () => ["span", { class: "chip" }, 0],
    });
    const chipSchema = new Schema({ nodes, marks: defaultSchema.spec.marks });
    const { a, b, sync } = setup({ schema: chipSchema });
    await tick();
    type(a, "ab");
    a.dispatch(a.state.tr.insert(3, chipSchema.node("chip", null, [chipSchema.text("XX")])));
    caretAt(a, 7); // after the chip
    type(a, "cd");
    sync();
    caretAt(b, 9); // after "cd"
    caretAt(a, 1);
    type(a, "Z");
    sync();
    expect(b.state.selection.head).toBe(10);
  });

  test("a selected node stays selected through a remote edit", async () => {
    const { a, b, sync } = setup();
    await tick();
    type(a, "pic ");
    a.dispatch(a.state.tr.insert(5, defaultSchema.node("image", { src: "x.png" })));
    sync();
    b.dispatch(b.state.tr.setSelection(NodeSelection.create(b.state.doc, 5)));
    caretAt(a, 1);
    type(a, "A ");
    sync();
    expect(b.state.selection).toBeInstanceOf(NodeSelection);
    expect((b.state.selection as InstanceType<typeof NodeSelection>).node.type.name).toBe("image");
  });
});

describe("ProseMirrorSyncPlugin with nested nodes", () => {
  // A table's rows and cells, with prosemirror-tables' content rules.
  const tableSchema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { group: "block", content: "inline*", toDOM: () => ["p", 0] },
      table: { group: "block", content: "row+", toDOM: () => ["table", ["tbody", 0]] },
      row: { content: "cell*", toDOM: () => ["tr", 0] },
      cell: { content: "block+", isolating: true, toDOM: () => ["td", 0] },
      text: { group: "inline" },
    },
  });
  const p = (t: string) => tableSchema.node("paragraph", null, t ? [tableSchema.text(t)] : []);
  const table = (rows: string[][]) =>
    tableSchema.node(
      "table",
      null,
      rows.map((r) => tableSchema.node("row", null, r.map((c) => tableSchema.node("cell", null, [p(c)])))),
    );
  const doc = (rows: string[][]) => tableSchema.node("doc", null, [p("Owners"), table(rows)]);

  // A model streaming a table: each write is the whole document again. Here
  // the next write fills the first cell and the second. The smallest
  // replace runs from inside one cell into the next, and ProseMirror does
  // not join across isolating cells: the row got a fourth cell.
  test("a document rewritten through a table reaches the editor exactly", async () => {
    const { a, docs } = setup({ schema: tableSchema });
    await tick();
    const steps = [
      doc([["Area", "Owner", "Due"], ["Rele", "", ""]]),
      doc([["Area", "Owner", "Due"], ["Release notes", "Ben", ""]]),
      doc([["Area", "Owner", "Due"], ["Release notes", "Ben", "Oct 25"], ["Sup", "", ""]]),
      doc([["Area", "Owner", "Due"], ["Release notes", "Ben", "Oct 25"], ["Support inbox", "TBD", "Oct 26"]]),
    ];
    for (const next of steps) {
      writeProseMirror(docs[1]!, "body", tableSchema, next);
      docs[1]!.commit();
      docs[0]!.import(docs[1]!.export({ mode: "update", from: docs[0]!.oplogVersion() }));
      await tick();
      expect(a.state.doc.toJSON()).toEqual(next.toJSON());
    }
  });

  test("a selected cell the change does not touch stays selected", async () => {
    const { a, docs } = setup({ schema: tableSchema });
    await tick();
    const write = async (next: Node) => {
      writeProseMirror(docs[1]!, "body", tableSchema, next);
      docs[1]!.commit();
      docs[0]!.import(docs[1]!.export({ mode: "update", from: docs[0]!.oplogVersion() }));
      await tick();
    };
    await write(doc([["Area", "Owner", "Due"], ["Rele", "", ""]]));
    let areaCell = -1;
    a.state.doc.descendants((node, pos) => {
      if (areaCell < 0 && node.type.name === "cell") areaCell = pos;
    });
    a.dispatch(a.state.tr.setSelection(NodeSelection.create(a.state.doc, areaCell)));
    await write(doc([["Area", "Owner", "Due"], ["Release notes", "Ben", ""]]));
    expect(a.state.selection).toBeInstanceOf(NodeSelection);
    expect(a.state.selection.from).toBe(areaCell);
    expect((a.state.selection as InstanceType<typeof NodeSelection>).node.textContent).toBe("Area");
  });

  test("a selected paragraph keeps its selection when the cells around it change", async () => {
    const { a, docs } = setup({ schema: tableSchema });
    await tick();
    const cells = (first: string, second: string) =>
      tableSchema.node("doc", null, [
        tableSchema.node("table", null, [
          tableSchema.node("row", null, [
            tableSchema.node("cell", null, [p("Keep"), p(first)]),
            tableSchema.node("cell", null, [p(second)]),
          ]),
        ]),
      ]);
    const write = async (next: Node) => {
      writeProseMirror(docs[1]!, "body", tableSchema, next);
      docs[1]!.commit();
      docs[0]!.import(docs[1]!.export({ mode: "update", from: docs[0]!.oplogVersion() }));
      await tick();
    };
    await write(cells("Rele", ""));
    // doc > table > row > cell > the "Keep" paragraph.
    a.dispatch(a.state.tr.setSelection(NodeSelection.create(a.state.doc, 3)));
    await write(cells("Release notes", "Ben"));
    expect(a.state.doc.toJSON()).toEqual(cells("Release notes", "Ben").toJSON());
    expect(a.state.selection).toBeInstanceOf(NodeSelection);
    expect((a.state.selection as InstanceType<typeof NodeSelection>).node.textContent).toBe("Keep");
  });

  test("typing in a paragraph still maps the selection, not the whole block", async () => {
    const { a, b, sync } = setup({ schema: tableSchema });
    await tick();
    b.dispatch(b.state.tr.replaceWith(0, b.state.doc.content.size, doc([["x", "y", "z"]]).content));
    sync();
    await tick();
    // Ada's caret after "Own".
    a.dispatch(a.state.tr.setSelection(TextSelection.create(a.state.doc, 4)));
    b.dispatch(b.state.tr.insertText("The ", 1));
    sync();
    await tick();
    expect(a.state.doc.child(0).textContent).toBe("The Owners");
    expect(a.state.selection.from).toBe(8);
  });
});
