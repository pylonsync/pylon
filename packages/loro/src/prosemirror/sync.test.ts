// ProseMirrorSyncPlugin with two real editors (happy-dom) on two LoroDocs.
// `sync()` stands in for the network: it moves each doc's new ops to the
// other, the way the outbox and the server broadcast do.
import { GlobalRegistrator } from "@happy-dom/global-registrator";
if (!globalThis.document) GlobalRegistrator.register();

import { afterEach, describe, expect, test } from "bun:test";
import { LoroDoc, LoroList, LoroMap, type ContainerID } from "loro-crdt";
import { undo } from "loro-prosemirror";
import { Schema } from "prosemirror-model";
import { schema as basic } from "prosemirror-schema-basic";
import { EditorState, TextSelection } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { configureProseMirrorMarks, findProseMirrorContainer } from "./index";
import { ProseMirrorSyncPlugin, ProseMirrorUndoPlugin } from "./sync";

const schema = new Schema({ nodes: basic.spec.nodes, marks: basic.spec.marks });
const tick = () => new Promise((r) => setTimeout(r, 0));

const views: EditorView[] = [];
afterEach(() => {
  for (const v of views.splice(0)) v.destroy();
});

/** A server doc with the field's container, and two clients synced to it. */
function setup() {
  // The empty document the server makes on insert (crates/crdt prosemirror.rs).
  const server = new LoroDoc();
  const body = server.getMap("row").setContainer("body", new LoroMap());
  body.set("nodeName", "doc");
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
  const [a, b] = [make(docs[0]), make(docs[1])];
  const sync = () => {
    for (const from of docs) {
      const bytes = from.export({ mode: "update", from: server.oplogVersion() });
      server.import(bytes);
    }
    const all = server.export({ mode: "snapshot" });
    for (const to of docs) to.import(all);
  };
  return { a, b, docs, sync };
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
