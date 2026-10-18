// Remote carets between two real editors (happy-dom) bound to two LoroDocs.
import { GlobalRegistrator } from "@happy-dom/global-registrator";
if (!globalThis.document) GlobalRegistrator.register();

import { afterEach, expect, test } from "bun:test";
import { LoroDoc, LoroList, LoroMap, type ContainerID } from "loro-crdt";
import { Schema } from "prosemirror-model";
import { schema as basic } from "prosemirror-schema-basic";
import { EditorState, TextSelection } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { proseMirrorCursorsKey, ProseMirrorCursorsPlugin, encodeSelection, setRemoteCursors, type EncodedSelection } from "./cursors";
import { configureProseMirrorMarks, findProseMirrorContainer } from "./index";
import { ProseMirrorSyncPlugin } from "./sync";

const schema = new Schema({ nodes: basic.spec.nodes, marks: basic.spec.marks });
const tick = () => new Promise((r) => setTimeout(r, 0));
const views: EditorView[] = [];
afterEach(() => {
  for (const v of views.splice(0)) v.destroy();
});

function setup() {
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
  const sent: EncodedSelection[] = [];
  const make = (doc: LoroDoc, onSelection?: (s: EncodedSelection) => void) => {
    const view = new EditorView(document.createElement("div"), {
      state: EditorState.create({
        schema,
        plugins: [ProseMirrorSyncPlugin({ doc, containerId }), ProseMirrorCursorsPlugin({ doc, containerId, onSelection })],
      }),
    });
    views.push(view);
    return view;
  };
  const a = make(docs[0], (s) => sent.push(s));
  const b = make(docs[1]);
  const sync = () => {
    for (const from of docs) server.import(from.export({ mode: "update", from: server.oplogVersion() }));
    const all = server.export({ mode: "snapshot" });
    for (const to of docs) to.import(all);
  };
  return { a, b, docs, containerId, sync, sent };
}

const type = (v: EditorView, s: string) => {
  for (const ch of s) v.dispatch(v.state.tr.insertText(ch));
};
const caretAt = (v: EditorView, pos: number) => v.dispatch(v.state.tr.setSelection(TextSelection.create(v.state.doc, pos)));
function decorationsOf(v: EditorView): Array<{ from: number; to: number; spec: { key?: string } }> {
  const plugin = v.state.plugins.find((p) => p.spec.key === proseMirrorCursorsKey)!;
  const set = plugin.props.decorations!.call(plugin, v.state) as { find(): Array<{ from: number; to: number; spec: { key?: string } }> };
  return set.find();
}
const caretPositions = (v: EditorView) =>
  decorationsOf(v)
    .filter((d) => d.spec.key?.startsWith("caret-"))
    .map((d) => d.from);

test("a remote caret appears where the other person's caret is, and stays on its words", async () => {
  const { a, b, docs, containerId, sync } = setup();
  await tick();
  type(a, "hello world");
  sync();
  caretAt(a, 7); // after "hello "
  const selection = encodeSelection(a.state, docs[0], containerId);
  setRemoteCursors(b, [{ id: "a", name: "Ann", color: "#2563eb", selection }]);
  expect(caretPositions(b)).toEqual([7]);
  // B types before Ann's caret: it moves with her words.
  caretAt(b, 1);
  type(b, "oh ");
  expect(caretPositions(b)).toEqual([10]);
});

test("a selection is drawn as a range", async () => {
  const { a, b, docs, containerId, sync } = setup();
  await tick();
  type(a, "select me");
  sync();
  a.dispatch(a.state.tr.setSelection(TextSelection.create(a.state.doc, 1, 7)));
  setRemoteCursors(b, [{ id: "a", name: "Ann", color: "#2563eb", selection: encodeSelection(a.state, docs[0], containerId) }]);
  expect(decorationsOf(b).some((d) => d.from === 1 && d.to === 7)).toBe(true);
});

test("no peers, no decorations", async () => {
  const { b } = setup();
  await tick();
  setRemoteCursors(b, []);
  expect(caretPositions(b)).toEqual([]);
});
