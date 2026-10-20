// Anchored ranges (comment anchors) between two real editors (happy-dom) bound to two LoroDocs.
import { GlobalRegistrator } from "@happy-dom/global-registrator";
if (!globalThis.document) GlobalRegistrator.register();

import { afterEach, expect, test } from "bun:test";
import { LoroDoc, LoroList, LoroMap, LoroText, type ContainerID } from "loro-crdt";
import { Schema } from "prosemirror-model";
import { schema as basic } from "prosemirror-schema-basic";
import { EditorState, TextSelection } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { configureProseMirrorMarks, findProseMirrorContainer, readProseMirror, writeProseMirror } from "./index";
import {
  resolveRangesInDoc,
  diffMatches,
  encodeRange,
  mapSpans,
  proseMirrorRangesKey,
  ProseMirrorRangesPlugin,
  resolveRanges,
  setAnchoredRanges,
  type AnchoredRange,
  type EncodedRange,
} from "./ranges";
import { undo } from "loro-prosemirror";
import { ProseMirrorSyncPlugin, ProseMirrorUndoPlugin } from "./sync";

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
  const reanchored: Array<{ id: string; range: EncodedRange }> = [];
  const make = (doc: LoroDoc) => {
    const view = new EditorView(document.createElement("div"), {
      state: EditorState.create({
        schema,
        plugins: [
          ProseMirrorSyncPlugin({ doc, containerId }),
          ProseMirrorUndoPlugin({ doc }),
          ProseMirrorRangesPlugin({ doc, containerId, onReanchor: (id, range) => reanchored.push({ id, range }) }),
        ],
      }),
    });
    views.push(view);
    return view;
  };
  const a = make(docs[0]);
  const b = make(docs[1]);
  const sync = () => {
    for (const from of docs) server.import(from.export({ mode: "update", from: server.oplogVersion() }));
    const all = server.export({ mode: "snapshot" });
    for (const to of docs) to.import(all);
  };
  return { a, b, docs, containerId, sync, reanchored };
}

const type = (v: EditorView, s: string) => {
  for (const ch of s) v.dispatch(v.state.tr.insertText(ch));
};
const caretAt = (v: EditorView, pos: number) => v.dispatch(v.state.tr.setSelection(TextSelection.create(v.state.doc, pos)));
function drawn(v: EditorView): Array<{ id: string; from: number; to: number }> {
  const plugin = v.state.plugins.find((p) => p.spec.key === proseMirrorRangesKey)!;
  const set = plugin.props.decorations!.call(plugin, v.state) as unknown as {
    find(): Array<{ from: number; to: number; spec: { rangeId: string } }>;
  };
  return set
    .find()
    .map((d) => ({ id: d.spec.rangeId, from: d.from, to: d.to }))
    .sort((x, y) => x.from - y.from || x.id.localeCompare(y.id));
}
const text = (v: EditorView, from: number, to: number) => v.state.doc.textBetween(from, to);
function drawnAfter(v: EditorView, ranges: AnchoredRange[]) {
  setAnchoredRanges(v, ranges);
  return drawn(v);
}

test("a range made in one editor is drawn on the same words in another", async () => {
  const { a, b, docs, containerId, sync } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10); // "quick"
  setAnchoredRanges(b, [{ id: "t1", ...anchor }]);
  expect(drawn(b)).toEqual([{ id: "t1", from: 5, to: 10 }]);
  expect(text(b, 5, 10)).toBe("quick");
});

test("a range stays on its words when text is added before it", async () => {
  const { a, b, docs, containerId, sync } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10);
  setAnchoredRanges(b, [{ id: "t1", ...anchor }]);
  caretAt(b, 1);
  type(b, "see ");
  const [r] = drawn(b);
  expect(text(b, r.from, r.to)).toBe("quick");
  // The other editor gets the same answer after sync.
  sync();
  const [ra] = resolveRanges(a.state, docs[0], containerId, [{ id: "t1", ...anchor }]);
  expect(ra.gone).toBe(false);
  expect(text(a, ra.from, ra.to)).toBe("quick");
});

test("a range grows when text is typed inside it", async () => {
  const { a, docs, containerId } = setup();
  await tick();
  type(a, "the quick fox");
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10);
  caretAt(a, 7);
  type(a, "-");
  const [r] = resolveRanges(a.state, docs[0], containerId, [{ id: "t1", ...anchor }]);
  expect(text(a, r.from, r.to)).toBe("qu-ick");
});

test("a range whose text is deleted is gone and is not drawn", async () => {
  const { a, b, docs, containerId, sync } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10);
  setAnchoredRanges(b, [{ id: "t1", ...anchor }]);
  b.dispatch(b.state.tr.delete(5, 11)); // "quick "
  expect(drawn(b)).toEqual([]);
  const [r] = resolveRanges(b.state, docs[1], containerId, [{ id: "t1", ...anchor }]);
  expect(r.gone).toBe(true);
});

test("ranges may overlap and keep their class and attributes", async () => {
  const { a, docs, containerId } = setup();
  await tick();
  type(a, "the quick fox");
  const ranges: AnchoredRange[] = [
    { id: "t1", ...encodeRange(a.state, docs[0], containerId, 1, 10), className: "thread" },
    { id: "t2", ...encodeRange(a.state, docs[0], containerId, 5, 14), attributes: { "data-thread-id": "t2" } },
  ];
  setAnchoredRanges(a, ranges);
  expect(drawn(a)).toEqual([
    { id: "t1", from: 1, to: 10 },
    { id: "t2", from: 5, to: 14 },
  ]);
  const plugin = a.state.plugins.find((p) => p.spec.key === proseMirrorRangesKey)!;
  const set = plugin.props.decorations!.call(plugin, a.state) as unknown as {
    find(): Array<{ spec: { rangeId: string }; type: { attrs: Record<string, string> } }>;
  };
  const attrs = Object.fromEntries(set.find().map((d) => [d.spec.rangeId, d.type.attrs]));
  expect(attrs.t1.class).toBe("thread");
  expect(attrs.t2.class).toBe("pylon-anchored-range");
  expect(attrs.t2["data-thread-id"]).toBe("t2");
});

test("a reversed range is stored in document order", async () => {
  const { a, docs, containerId } = setup();
  await tick();
  type(a, "the quick fox");
  const anchor = encodeRange(a.state, docs[0], containerId, 10, 5);
  const [r] = resolveRanges(a.state, docs[0], containerId, [{ id: "t1", ...anchor }]);
  expect([r.from, r.to]).toEqual([5, 10]);
});

test("bad or missing cursors resolve as gone, not as an error", async () => {
  const { a, docs, containerId } = setup();
  await tick();
  type(a, "hello");
  const out = resolveRanges(a.state, docs[0], containerId, [
    { id: "x", from: "not a cursor", to: "!!" },
    { id: "y", from: null, to: null },
  ]);
  expect(out.map((r) => r.gone)).toEqual([true, true]);
  setAnchoredRanges(a, [{ id: "x", from: "not a cursor", to: "!!" }]);
  expect(drawn(a)).toEqual([]);
});

const covered = (v: EditorView, id = "t1") => {
  const r = drawn(v).find((d) => d.id === id);
  return r ? v.state.doc.textBetween(r.from, r.to, "|") : null;
};

test("text typed just before or just after a range stays out of it", async () => {
  const { a, docs, containerId } = setup();
  await tick();
  type(a, "the quick fox");
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10);
  caretAt(a, 5);
  type(a, "new ");
  caretAt(a, 14); // after "quick"
  type(a, "est");
  const [r] = resolveRanges(a.state, docs[0], containerId, [{ id: "t1", ...anchor }]);
  expect(text(a, r.from, r.to)).toBe("quick");
});

test("the editor that splits a paragraph keeps the range and gives new cursors to store", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10);
  setAnchoredRanges(a, [{ id: "t1", ...anchor }]);
  a.dispatch(a.state.tr.split(5));
  expect(covered(a)).toBe("quick");
  expect(reanchored.map((r) => r.id)).toEqual(["t1"]);
  // The other editor, given the stored cursors, finds it too.
  sync();
  await tick();
  setAnchoredRanges(b, [{ id: "t1", ...reanchored[0].range }]);
  expect(covered(b)).toBe("quick");
  // Once the app sets the new cursors, nothing more to store.
  setAnchoredRanges(a, [{ id: "t1", ...reanchored[0].range }]);
  caretAt(a, 1);
  type(a, "x");
  expect(covered(a)).toBe("quick");
  expect(reanchored.length).toBe(1);
});

test("a range split across two paragraphs keeps both parts", async () => {
  const { a, docs, containerId } = setup();
  await tick();
  type(a, "the quick fox");
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  a.dispatch(a.state.tr.split(7)); // "the qu" | "ick fox"
  expect(covered(a)).toBe("qu|ick");
});

test("text typed inside a range, then a split before it, keeps the whole range", async () => {
  const { a, docs, containerId } = setup();
  await tick();
  type(a, "the quick fox");
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  caretAt(a, 7);
  type(a, "-");
  a.dispatch(a.state.tr.split(5));
  expect(covered(a)).toBe("qu-ick");
});

test("joining paragraphs keeps a range in the second one", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the ");
  a.dispatch(a.state.tr.split(5));
  type(a, "quick fox");
  // "the " | "quick fox": quick is at 7..12.
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 7, 12) }]);
  expect(covered(a)).toBe("quick");
  a.dispatch(a.state.tr.join(6));
  expect(a.state.doc.childCount).toBe(1);
  expect(covered(a)).toBe("quick");
  expect(reanchored.length).toBeGreaterThan(0);
});

test("a range whose text is replaced is gone, even when the same words are elsewhere", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox quick");
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  a.dispatch(a.state.tr.insertText("slow", 5, 10));
  expect(covered(a)).toBeNull();
  expect(reanchored).toEqual([]);
});

test("a range keeps what is left when part of its text is deleted", async () => {
  const { a, docs, containerId } = setup();
  await tick();
  type(a, "the quick fox");
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10);
  setAnchoredRanges(a, [{ id: "t1", ...anchor }]);
  a.dispatch(a.state.tr.delete(5, 8)); // "qui"
  expect(covered(a)).toBe("ck");
  const [r] = resolveRanges(a.state, docs[0], containerId, [{ id: "t1", ...anchor }]);
  expect(text(a, r.from, r.to)).toBe("ck");
});

test("a selection from the end of one paragraph starts at the next one's text", async () => {
  const { a, docs, containerId } = setup();
  await tick();
  type(a, "one");
  a.dispatch(a.state.tr.split(4));
  type(a, "two");
  const anchor = encodeRange(a.state, docs[0], containerId, 4, 9);
  const [r] = resolveRanges(a.state, docs[0], containerId, [{ id: "t1", ...anchor }]);
  expect(text(a, r.from, r.to)).toBe("two");
  expect(encodeRange(a.state, docs[0], containerId, 1, 1)).toEqual({ from: null, to: null });
});

test("a remote change that leaves the same text redraws the range", async () => {
  const { a, docs, containerId, sync } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  expect(drawn(a)).toEqual([{ id: "t1", from: 5, to: 10 }]);
  // B deletes the text and types it again as new characters, in one commit.
  const root = docs[1].getContainerById(containerId) as LoroMap;
  const para = (root.get("children") as LoroList).get(0) as LoroMap;
  const t = (para.get("children") as LoroList).get(0) as LoroText;
  t.delete(0, t.length);
  t.insert(0, "the quick fox");
  docs[1].commit();
  sync();
  await tick();
  await tick();
  expect(a.state.doc.textContent).toBe("the quick fox");
  expect(drawn(a)).toEqual([]);
});

test("mapSpans cuts deleted text out and keeps edges inside", () => {
  const state = EditorState.create({ schema, doc: schema.node("doc", null, [schema.node("paragraph", null, [schema.text("the quick fox")])]) });
  const span = [[5, 10]] as const;
  expect(mapSpans(span, state.tr.insertText("slow", 5, 10))).toEqual([]);
  expect(mapSpans(span, state.tr.delete(5, 8))).toEqual([[5, 7]]);
  expect(mapSpans(span, state.tr.insertText("new ", 5))).toEqual([[9, 14]]);
  expect(mapSpans(span, state.tr.insertText("x", 10))).toEqual([[5, 10]]);
  expect(mapSpans(span, state.tr.insertText("x", 7))).toEqual([[5, 11]]);
});

test("a local edit, then a remote change that leaves the same text, still drops the range", async () => {
  const { a, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  caretAt(a, 14);
  type(a, "!");
  sync();
  await tick();
  // B deletes "the quick fox" and types it again as new characters.
  const root = docs[1].getContainerById(containerId) as LoroMap;
  const para = (root.get("children") as LoroList).get(0) as LoroMap;
  const t = (para.get("children") as LoroList).get(0) as LoroText;
  t.delete(0, 13);
  t.insert(0, "the quick fox");
  docs[1].commit();
  sync();
  await tick();
  await tick();
  expect(a.state.doc.textContent).toBe("the quick fox!");
  expect(covered(a)).toBeNull();
  expect(reanchored).toEqual([]);
});

test("undoing a split keeps the range and gives new cursors again", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  await new Promise((r) => setTimeout(r, 1100)); // a separate undo step
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  a.dispatch(a.state.tr.split(5));
  expect(covered(a)).toBe("quick");
  undo(a.state, a.dispatch);
  await tick();
  expect(a.state.doc.childCount).toBe(1);
  expect(covered(a)).toBe("quick");
  expect(reanchored.length).toBe(2);
  const [r] = resolveRanges(a.state, docs[0], containerId, [{ id: "t1", ...reanchored[1].range }]);
  expect(text(a, r.from, r.to)).toBe("quick");
});

test("saves that come back out of order do not undo newer cursors", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  a.dispatch(a.state.tr.split(5)); // "the " | "quick fox"
  a.dispatch(a.state.tr.split(9)); // "the " | "qu" | "ick fox"
  expect(covered(a)).toBe("qu|ick");
  expect(reanchored.length).toBe(2);
  // The first save comes back last.
  setAnchoredRanges(a, [{ id: "t1", ...reanchored[0].range }]);
  expect(covered(a)).toBe("qu|ick");
  setAnchoredRanges(a, [{ id: "t1", ...reanchored[1].range }]);
  expect(covered(a)).toBe("qu|ick");
});

test("a view update without a new state does not store the cursors again", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  a.dispatch(a.state.tr.split(5));
  expect(reanchored.length).toBe(1);
  a.setProps({ editable: () => true });
  a.setProps({ editable: () => true });
  expect(reanchored.length).toBe(1);
});

test("new cursors from the app replace the followed range", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  caretAt(a, 14);
  type(a, "!");
  setAnchoredRanges(a, [{ id: "t1", from: null, to: null }]);
  caretAt(a, 1);
  type(a, "x");
  expect(covered(a)).toBeNull();
  expect(reanchored).toEqual([]);
});

test("a remote change that arrives as one replace over the range shows again with the new cursors", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10);
  setAnchoredRanges(a, [{ id: "t1", ...anchor }]);
  setAnchoredRanges(b, [{ id: "t1", ...anchor }]);
  // A splits before "quick" and types after the text, both before B imports.
  a.dispatch(a.state.tr.split(5));
  caretAt(a, a.state.doc.content.size - 1);
  type(a, "!");
  sync();
  await tick();
  await tick();
  expect(b.state.doc.textBetween(0, b.state.doc.content.size, "|")).toBe("the |quick fox!");
  // B's cursors name characters the split replaced, and the change reached B
  // as one replace over them: B waits for A's cursors.
  setAnchoredRanges(b, [{ id: "t1", ...reanchored.at(-1)!.range }]);
  expect(covered(b)).toBe("quick");
});

test("an editor stores new cursors only for its own last edit", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10);
  setAnchoredRanges(a, [{ id: "t1", ...anchor }]);
  setAnchoredRanges(b, [{ id: "t1", ...anchor }]);
  caretAt(b, b.state.doc.content.size - 1);
  type(b, "!");
  sync();
  await tick();
  const before = reanchored.length;
  a.dispatch(a.state.tr.split(5));
  caretAt(a, 9); // after "qu"
  type(a, "-");
  const fromA = reanchored.length - before;
  sync();
  await tick();
  await tick();
  // Only A's callbacks: B followed the range to draw it.
  expect(reanchored.length - before).toBe(fromA);
  expect(covered(a)).toBe("qu-ick");
});

test("undoing a split together with typing keeps the range", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  await new Promise((r) => setTimeout(r, 1100)); // a separate undo step
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  a.dispatch(a.state.tr.split(5));
  caretAt(a, a.state.doc.content.size - 1);
  type(a, "!"); // the same undo step
  undo(a.state, a.dispatch);
  await tick();
  expect(a.state.doc.textContent).toBe("the quick fox");
  expect(covered(a)).toBe("quick");
  const [r] = resolveRanges(a.state, docs[0], containerId, [{ id: "t1", ...reanchored.at(-1)!.range }]);
  expect(text(a, r.from, r.to)).toBe("quick");
});

test("someone else's replace of the range's words hides it", async () => {
  const { a, b, docs, containerId, sync } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  setAnchoredRanges(b, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  a.dispatch(a.state.tr.insertText("slow", 5, 10));
  sync();
  await tick();
  await tick();
  expect(b.state.doc.textContent).toBe("the slow fox");
  expect(covered(b)).toBeNull();
});

test("diffMatches pairs equal entries of a shortest edit script", () => {
  const chars = (s: string) => [...s];
  expect(diffMatches(chars("abc"), chars("abc"))).toEqual([[0, 0], [1, 1], [2, 2]]);
  expect(diffMatches(chars("ab|cd"), chars("abcd!"))).toEqual([[0, 0], [1, 1], [3, 2], [4, 3]]);
  expect(diffMatches(chars("abc"), chars(""))).toEqual([]);
  expect(diffMatches(chars("kitten"), chars("sitting"))!.map(([, j]) => "sitting"[j]).join("")).toBe("ittn");
  const long = "x".repeat(1000);
  expect(diffMatches(chars(long), chars("y".repeat(1000)))).toBeNull();
});

test("Loro calls container listeners before doc listeners on import (the plugin relies on it)", () => {
  const a = new LoroDoc();
  const t = a.getMap("row").setContainer("body", new LoroMap()).setContainer("t", new LoroText());
  a.commit();
  const b = new LoroDoc();
  b.import(a.export({ mode: "snapshot" }));
  const order: string[] = [];
  b.subscribe(() => order.push("doc"));
  (b.getMap("row").get("body") as LoroMap).subscribe(() => order.push("container"));
  t.insert(0, "x");
  a.commit();
  b.import(a.export({ mode: "update", from: b.oplogVersion() }));
  expect(order).toEqual(["container", "doc"]);
});

const retypeFirstText = (d: LoroDoc, containerId: ContainerID, paragraph = 0) => {
  const root = d.getContainerById(containerId) as LoroMap;
  const t = ((root.get("children") as LoroList).get(paragraph) as LoroMap).get("children") as LoroList;
  const text = t.get(0) as LoroText;
  const value = text.toString();
  text.delete(0, value.length);
  text.insert(0, value);
  d.commit();
};

test("a range drawn by its cursors is redrawn when a remote import retypes its text", async () => {
  const { a, b, docs, containerId, sync } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  setAnchoredRanges(b, [{ id: "t1", ...encodeRange(b.state, docs[1], containerId, 5, 10) }]);
  expect(covered(b)).toBe("quick");
  retypeFirstText(docs[0], containerId);
  sync();
  await tick();
  await tick();
  const plugin = b.state.plugins.find((p) => p.spec.key === proseMirrorRangesKey)!;
  const set = plugin.props.decorations!.call(plugin, b.state) as unknown as { find(): unknown[] };
  expect(set.find()).toEqual([]);
  expect(b.dom.querySelector(".pylon-anchored-range")).toBeNull();
  void a;
});

test("an import that changes another paragraph does not hide a retype in the same tick", async () => {
  const { a, b, docs, containerId, sync } = setup();
  await tick();
  type(a, "one");
  a.dispatch(a.state.tr.split(4));
  type(a, "the quick fox");
  sync();
  await tick();
  // "one" | "the quick fox": quick is at 10..15. B follows it through A's split before it.
  setAnchoredRanges(b, [{ id: "t1", ...encodeRange(b.state, docs[1], containerId, 10, 15) }]);
  expect(covered(b)).toBe("quick");
  // Two imports before the microtask: one types in the first paragraph, one retypes the second.
  caretAt(a, 4);
  type(a, "!");
  sync();
  retypeFirstText(docs[0], containerId, 1);
  sync();
  await tick();
  await tick();
  expect(covered(b)).toBeNull();
});

test("diffMatches handles long common runs and a pure insert", () => {
  const big = Array.from({ length: 150_000 }, (_, i) => String.fromCharCode(97 + (i % 26)));
  const out = diffMatches(["<", ...big, ">"], ["[", ...big, "]"]);
  expect(out?.length).toBe(150_000);
  expect(diffMatches(["a"], ["a", ...Array(401).fill("b")])).toEqual([[0, 0]]);
});

test("someone else's split hides the range here until their new cursors arrive", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10);
  setAnchoredRanges(a, [{ id: "t1", ...anchor }]);
  setAnchoredRanges(b, [{ id: "t1", ...anchor }]);
  a.dispatch(a.state.tr.split(5));
  expect(reanchored.length).toBe(1);
  sync();
  await tick();
  await tick();
  expect(b.state.doc.childCount).toBe(2);
  // B's cursors name the characters the split replaced: never drawn on others.
  expect(covered(b)).toBeNull();
  // B stores nothing; A's cursors bring it back.
  expect(reanchored.length).toBe(1);
  setAnchoredRanges(b, [{ id: "t1", ...reanchored[0].range }]);
  expect(covered(b)).toBe("quick");
});

test("deleting the first of two equal words takes the range with it", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "quick quick");
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 1, 6) }]);
  a.dispatch(a.state.tr.delete(1, 7));
  expect(a.state.doc.textContent).toBe("quick");
  expect(covered(a)).toBeNull();
  expect(reanchored).toEqual([{ id: "t1", range: { from: null, to: null } }]);
});

test("a local edit right after a retype import does not store cursors on the new characters", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  setAnchoredRanges(b, [{ id: "t1", ...encodeRange(b.state, docs[1], containerId, 5, 10) }]);
  const root = docs[0].getContainerById(containerId) as LoroMap;
  const t = ((root.get("children") as LoroList).get(0) as LoroMap).get("children") as LoroList;
  const text = t.get(0) as LoroText;
  text.delete(0, 13);
  text.insert(0, "the quick fox");
  docs[0].commit();
  sync(); // B imports; the refresh is still queued
  caretAt(b, 14);
  type(b, "!");
  await tick();
  await tick();
  expect(reanchored).toEqual([]);
  expect(covered(b)).toBeNull();
});

test("an import right after an undo is someone else's change", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  const anchor = encodeRange(a.state, docs[0], containerId, 5, 10);
  setAnchoredRanges(b, [{ id: "t1", ...anchor }]);
  await new Promise((r) => setTimeout(r, 1100));
  caretAt(b, 1);
  type(b, "x");
  await new Promise((r) => setTimeout(r, 1100));
  undo(b.state, b.dispatch); // B's undo; loro-prosemirror's undo flag stays set until a timer
  a.dispatch(a.state.tr.split(5));
  sync(); // A's split reaches B inside that window
  expect(b.state.doc.childCount).toBe(2);
  expect(covered(b)).toBeNull();
  expect(reanchored).toEqual([]);
});

test("deleting a range's text where the binding keeps its characters clears its cursors", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "quick quick");
  sync();
  await tick();
  const anchor = encodeRange(a.state, docs[0], containerId, 1, 6);
  setAnchoredRanges(a, [{ id: "t1", ...anchor }]);
  a.dispatch(a.state.tr.delete(1, 7));
  expect(reanchored).toEqual([{ id: "t1", range: { from: null, to: null } }]);
  // The app stores the cleared anchor; every editor then hides it.
  sync();
  await tick();
  setAnchoredRanges(b, [{ id: "t1", from: null, to: null }]);
  setAnchoredRanges(a, [{ id: "t1", from: null, to: null }]);
  expect(covered(a)).toBeNull();
  expect(covered(b)).toBeNull();
});

test("no new cursors are stored while an import waits for its refresh", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  setAnchoredRanges(b, [{ id: "t1", ...encodeRange(b.state, docs[1], containerId, 5, 10) }]);
  caretAt(b, 14);
  type(b, "!"); // B now follows the range
  sync();
  await tick();
  const root = docs[0].getContainerById(containerId) as LoroMap;
  const t = (((root.get("children") as LoroList).get(0) as LoroMap).get("children") as LoroList).get(0) as LoroText;
  const value = t.toString();
  t.delete(0, value.length);
  t.insert(0, value);
  docs[0].commit();
  sync();
  caretAt(b, 2); // a selection change before the refresh
  await tick();
  await tick();
  expect(reanchored).toEqual([]);
  expect(covered(b)).toBeNull();
});

test("a selection change between a retype import and its refresh does not bring the range back", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  setAnchoredRanges(b, [{ id: "t1", ...encodeRange(b.state, docs[1], containerId, 5, 10) }]);
  caretAt(b, 14);
  type(b, "!");
  sync();
  await tick();
  const root = docs[0].getContainerById(containerId) as LoroMap;
  const t = (((root.get("children") as LoroList).get(0) as LoroMap).get("children") as LoroList).get(0) as LoroText;
  const value = t.toString();
  t.delete(0, value.length);
  t.insert(0, value);
  docs[0].commit();
  sync();
  caretAt(b, 2);
  await tick();
  await tick();
  caretAt(b, 3);
  expect(covered(b)).toBeNull();
  expect(reanchored).toEqual([]);
});

test("a split right after an import still gives new cursors", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  setAnchoredRanges(b, [{ id: "t1", ...encodeRange(b.state, docs[1], containerId, 5, 10) }]);
  caretAt(a, 14);
  type(a, "!");
  sync(); // B imports; then splits at once
  b.dispatch(b.state.tr.split(5));
  expect(covered(b)).toBe("quick");
  expect(reanchored.map((r) => r.id)).toEqual(["t1"]);
  await tick();
  await tick();
  expect(covered(b)).toBe("quick");
});

test("a followed range whose edge text is deleted gives new cursors once", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  a.dispatch(a.state.tr.split(7)); // "the qu" | "ick fox"
  a.dispatch(a.state.tr.delete(5, 7)); // "qu"
  expect(covered(a)).toBe("ick");
  const count = reanchored.length;
  a.setProps({ editable: () => true });
  a.setProps({ editable: () => true });
  expect(reanchored.length).toBe(count);
});

test("a deleted range whose cursors still find part of the text is cleared", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "aaaaa");
  const anchor = encodeRange(a.state, docs[0], containerId, 1, 4);
  setAnchoredRanges(a, [{ id: "t1", ...anchor }]);
  a.dispatch(a.state.tr.delete(1, 4));
  expect(covered(a)).toBeNull();
  expect(reanchored).toEqual([{ id: "t1", range: { from: null, to: null } }]);
});

test("app code writing the field directly is not taken for an undo", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  await tick();
  // Replace the text with new characters, outside the editor.
  const root = docs[0].getContainerById(containerId) as LoroMap;
  const t = (((root.get("children") as LoroList).get(0) as LoroMap).get("children") as LoroList).get(0) as LoroText;
  t.delete(0, t.length);
  t.insert(0, "the quick fox!");
  docs[0].commit();
  await tick();
  expect(a.state.doc.textContent).toBe("the quick fox!");
  expect(covered(a)).toBeNull();
  expect(reanchored).toEqual([]);
});

test("reading the decorations before the refresh does not cancel it", async () => {
  const { a, b, docs, containerId, sync } = setup();
  await tick();
  type(a, "the quick fox");
  sync();
  await tick();
  setAnchoredRanges(b, [{ id: "t1", ...encodeRange(b.state, docs[1], containerId, 5, 10) }]);
  const root = docs[0].getContainerById(containerId) as LoroMap;
  const t = (((root.get("children") as LoroList).get(0) as LoroMap).get("children") as LoroList).get(0) as LoroText;
  t.delete(0, 13);
  t.insert(0, "the quick fox");
  docs[0].commit();
  sync();
  drawn(b); // reads the decorations before the microtask
  await tick();
  await tick();
  expect(b.dom.querySelector(".pylon-anchored-range")).toBeNull();
});

test("app code retyping the field's text drops a followed range", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  caretAt(a, 14);
  type(a, "!"); // a followed range
  const root = docs[0].getContainerById(containerId) as LoroMap;
  const t = (((root.get("children") as LoroList).get(0) as LoroMap).get("children") as LoroList).get(0) as LoroText;
  const value = t.toString();
  t.delete(0, value.length);
  t.insert(0, value);
  docs[0].commit();
  await tick();
  await tick();
  caretAt(a, 2);
  expect(covered(a)).toBeNull();
  expect(reanchored).toEqual([]);
});

test("an undo that rewrites text as it was keeps the range and gives new cursors", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  await new Promise((r) => setTimeout(r, 1100));
  a.dispatch(a.state.tr.split(5));
  a.dispatch(a.state.tr.join(6)); // the same undo step: the text is as it was
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  expect(covered(a)).toBe("quick");
  undo(a.state, a.dispatch);
  await tick();
  await tick();
  expect(a.state.doc.textContent).toBe("the quick fox");
  expect(covered(a)).toBe("quick");
  const [r] = resolveRanges(a.state, docs[0], containerId, [{ id: "t1", ...reanchored.at(-1)!.range }]);
  expect(text(a, r.from, r.to)).toBe("quick");
});

test("an in-place undo keeps the range through a selection change right after", async () => {
  const { a, docs, containerId, reanchored } = setup();
  await tick();
  type(a, "the quick fox");
  await new Promise((r) => setTimeout(r, 1100));
  a.dispatch(a.state.tr.split(5));
  a.dispatch(a.state.tr.join(6));
  setAnchoredRanges(a, [{ id: "t1", ...encodeRange(a.state, docs[0], containerId, 5, 10) }]);
  undo(a.state, a.dispatch);
  caretAt(a, 2);
  await tick();
  await tick();
  expect(covered(a)).toBe("quick");
  expect(reanchored.length).toBe(1);
});

test("an in-place undo after an outside retype does not keep the retyped range", async () => {
  const { a, b, docs, containerId, sync, reanchored } = setup();
  await tick();
  type(b, "the quick fox");
  b.dispatch(b.state.tr.split(14));
  type(b, "two words");
  sync();
  await tick();
  await new Promise((r) => setTimeout(r, 1100));
  // B's undo step: split and join the second paragraph, leaving it as it was.
  b.dispatch(b.state.tr.split(19));
  b.dispatch(b.state.tr.join(20));
  setAnchoredRanges(b, [{ id: "t1", ...encodeRange(b.state, docs[1], containerId, 5, 10) }]);
  // A retypes the first paragraph as it was; B undoes before the refresh.
  const root = docs[0].getContainerById(containerId) as LoroMap;
  const t = (((root.get("children") as LoroList).get(0) as LoroMap).get("children") as LoroList).get(0) as LoroText;
  const value = t.toString();
  t.delete(0, value.length);
  t.insert(0, value);
  docs[0].commit();
  sync();
  undo(b.state, b.dispatch);
  await tick();
  await tick();
  expect(covered(b)).toBeNull();
  expect(reanchored).toEqual([]);
  void a;
});

// A server function has no editor: it reads the document from the doc.
test("resolveRangesInDoc finds a range in the doc as it is now, or says it is gone", () => {
  const doc = new LoroDoc();
  const body = doc.getMap("row").setContainer("body", new LoroMap());
  body.setContainer("attributes", new LoroMap());
  body.setContainer("children", new LoroList());
  doc.commit();
  configureProseMirrorMarks(doc, schema);
  const p = (t: string) => schema.node("paragraph", null, [schema.text(t)]);
  writeProseMirror(doc, "body", schema, schema.node("doc", null, [p("Hello world"), p("Second")]));
  doc.commit();
  const containerId = findProseMirrorContainer(doc, "body")!;
  const state = EditorState.create({ schema, doc: readProseMirror(doc, "body", schema)! });
  // "world": after "Hello " in the first paragraph (position 1 is its start).
  const anchor = encodeRange(state, doc, containerId, 7, 12);
  const text = (r: { from: number; to: number }) => readProseMirror(doc, "body", schema)!.textBetween(r.from, r.to);

  writeProseMirror(doc, "body", schema, schema.node("doc", null, [p("Oh, hello world"), p("Second")]));
  doc.commit();
  const [moved] = resolveRangesInDoc(doc, "body", schema, [{ id: "a", ...anchor }]);
  expect(moved!.gone).toBe(false);
  expect(text(moved!)).toBe("world");

  writeProseMirror(doc, "body", schema, schema.node("doc", null, [p("Oh, hello"), p("Second")]));
  doc.commit();
  expect(resolveRangesInDoc(doc, "body", schema, [{ id: "a", ...anchor }])[0]!.gone).toBe(true);
  // A doc with no document in the field.
  expect(resolveRangesInDoc(new LoroDoc(), "body", schema, [{ id: "a", ...anchor }])[0]!.gone).toBe(true);
});
