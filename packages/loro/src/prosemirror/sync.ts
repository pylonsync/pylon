/**
 * ProseMirrorSyncPlugin: binds a ProseMirror editor to a ProseMirror field
 * of a row's Loro doc. A drop-in replacement for loro-prosemirror's
 * LoroSyncPlugin (same plugin key and state, so LoroUndoPlugin works with
 * it), with two fixes for people typing at the same time:
 *
 * - A remote change is applied as one replace of the range that changed,
 *   not of the whole document. ProseMirror maps the selection, decorations
 *   and plugin state through it.
 * - The caret is kept as Loro cursors, taken after every editor update,
 *   when the editor and the doc agree. LoroSyncPlugin took them after the
 *   import had already changed the doc, replaced the whole document (which
 *   moves the caret to its end), and put the caret back in a setTimeout:
 *   text typed before that timer ran went to the end of the document, and
 *   two people typing at one spot interleaved.
 *
 * The caret binds to the character on its left. A remote insert at the
 * caret's spot then lands after it, and the next character this person
 * types continues their own run, which is how Loro orders concurrent runs.
 *
 * Ported in part from loro-prosemirror 0.4.4 (MIT, Loro contributors):
 * the cursor-to-position walk and the change bookkeeping.
 */
import {
  type ContainerID,
  Cursor,
  type LoroDoc,
  LoroList,
  LoroMap,
  LoroText,
  UndoManager,
} from "loro-crdt";
import {
  CHILDREN_KEY,
  createNodeFromLoroObj,
  LoroUndoPlugin,
  loroSyncPluginKey,
  loroUndoPluginKey,
  updateLoroToPmState,
} from "loro-prosemirror";
import type { Node } from "prosemirror-model";
import { Plugin, TextSelection, type EditorState, type Transaction } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";

type Mapping = Map<ContainerID, Node | Node[]>;

interface SyncState {
  doc: LoroDoc;
  mapping: Mapping;
  changedBy: "local" | "import" | "checkout";
  containerId?: ContainerID;
  snapshot?: unknown;
  docSubscription?: () => void;
}

interface Cursors {
  anchor: Cursor | null;
  head: Cursor | null;
}

export interface ProseMirrorSyncPluginProps {
  doc: LoroDoc;
  /** The field's container, from useProseMirrorDoc or findProseMirrorContainer. */
  containerId: ContainerID;
}

export function ProseMirrorSyncPlugin({ doc, containerId }: ProseMirrorSyncPluginProps): Plugin {
  // True while this editor writes to the doc. Loro reports a commit to
  // subscribers synchronously, in the middle of this editor's own dispatch;
  // those events are this editor's own edits. Every other change (another
  // editor on the same doc, app code, an undo, the server) is applied.
  let writing = false;
  const write = (fn: () => void) => {
    writing = true;
    try {
      fn();
    } finally {
      writing = false;
    }
  };
  return new Plugin({
    key: loroSyncPluginKey,
    props: {
      editable: (state) => (loroSyncPluginKey.getState(state) as SyncState | undefined)?.snapshot == null,
    },
    state: {
      init: (): SyncState => ({ doc, mapping: new Map(), changedBy: "local", containerId }),
      apply: (tr, state: SyncState, oldEditorState, newEditorState) => {
        const meta = tr.getMeta(loroSyncPluginKey) as { type: string; state?: Partial<SyncState> } | undefined;
        const undoState = loroUndoPluginKey.getState(oldEditorState) as
          | { isUndoing?: { current: boolean } }
          | undefined;
        state.changedBy = meta?.type === "non-local-updates" ? "import" : "local";
        switch (meta?.type) {
          case "doc-changed":
            if (!undoState?.isUndoing?.current) {
              write(() => updateLoroToPmState(doc as never, state.mapping as never, newEditorState, containerId));
            }
            break;
          case "update-state": {
            state = { ...state, ...meta.state };
            const committing = state.doc;
            write(() => committing.commit({ origin: "sys:init", timestamp: Date.now() }));
            break;
          }
        }
        return state;
      },
    },
    // Only this editor's own changes are written to the doc. A change that
    // came from the doc is there already, and the empty document an editor
    // shows for an emptied field must not be written back: every editor
    // would add its own.
    appendTransaction: (transactions, _old, newState) =>
      transactions.some(
        (tr) => tr.docChanged && (tr.getMeta(loroSyncPluginKey) as { type?: string } | undefined)?.type !== "non-local-updates",
      )
        ? newState.tr.setMeta(loroSyncPluginKey, { type: "doc-changed" })
        : null,
    view: (view) => {
      let cursors: Cursors = { anchor: null, head: null };
      let unsubscribe: (() => void) | null = null;
      const remember = () => {
        cursors = selectionToCursors(view.state, doc, containerId);
      };
      const timer = setTimeout(() => {
        if (view.isDestroyed) return;
        unsubscribe = doc.getContainerById(containerId)!.subscribe((event) => {
          if (writing) return;
          applyRemote(view, doc, containerId, event, cursors);
          remember();
        });
        // A new document has no top node name yet; the server leaves it to
        // the schema. Write it now, as setup (`sys:init`, which undo skips),
        // so the person's first edit is an edit they can undo.
        const root = doc.getContainerById(containerId) as LoroMap;
        if (root.get("nodeName") == null) {
          write(() => {
            root.set("nodeName", view.state.schema.topNodeType.name);
            doc.commit({ origin: "sys:init" });
          });
        }
        loadInitial(view, doc, containerId);
        remember();
      }, 0);
      return {
        update: () => {
          const state = loroSyncPluginKey.getState(view.state) as SyncState | undefined;
          // Only when the editor and the doc agree: after a local change the
          // apply above wrote it to Loro synchronously.
          if (state) remember();
        },
        destroy: () => {
          clearTimeout(timer);
          unsubscribe?.();
        },
      };
    },
  });
}

/** Edits closer together than this are one undo step. */
export const UNDO_MERGE_MS = 500;

/**
 * Undo for a document bound with ProseMirrorSyncPlugin: LoroUndoPlugin with
 * typing grouped into steps (Loro's default makes every character a step),
 * and without the binding's first load, which is not this person's edit.
 * It undoes only this person's edits.
 */
export function ProseMirrorUndoPlugin({ doc }: { doc: LoroDoc }): Plugin {
  const undoManager = new UndoManager(doc, {
    mergeInterval: UNDO_MERGE_MS,
    excludeOriginPrefixes: ["sys:init"],
  });
  return LoroUndoPlugin({ doc: doc as never, undoManager: undoManager as never });
}

function field(doc: LoroDoc, containerId: ContainerID): LoroMap {
  return doc.getContainerById(containerId) as LoroMap;
}

/**
 * The browser's selection in the editor, when it differs from the editor
 * state's and the window has focus. A click or an arrow key moves the DOM
 * selection first, and the editor reads it a little later; a remote change
 * in between would put the caret back where it was.
 */
function domSelectionIfMoved(view: EditorView): { anchor: number; head: number } | null {
  // A window without focus keeps a stale DOM selection: the editor does not
  // write its selection to the DOM while it is in the background.
  if (!view.hasFocus() || !view.dom.ownerDocument.hasFocus()) return null;
  const sel = (view.root as Document).getSelection?.();
  if (!sel?.anchorNode || !sel.focusNode || !view.dom.contains(sel.anchorNode)) return null;
  try {
    const anchor = view.posAtDOM(sel.anchorNode, sel.anchorOffset);
    const head = view.posAtDOM(sel.focusNode, sel.focusOffset);
    const { selection } = view.state;
    return anchor === selection.anchor && head === selection.head ? null : { anchor, head };
  } catch {
    return null;
  }
}

/** True for a document with no top node or no children yet. */
function isEmpty(map: LoroMap): boolean {
  if (map.get("nodeName") == null) return true;
  const children = map.get(CHILDREN_KEY);
  return !(children instanceof LoroList) || children.length === 0;
}

/** Fill the editor from the doc on mount (what LoroSyncPlugin's init does). */
function loadInitial(view: EditorView, doc: LoroDoc, containerId: ContainerID): void {
  const mapping: Mapping = new Map();
  const map = field(doc, containerId);
  let tr = view.state.tr;
  const node = isEmpty(map)
    ? null
    : (createNodeFromLoroObj(view.state.schema, map as never, mapping as never) as Node);
  // An empty document (the server's, before anyone typed) keeps the editor's
  // starting content; the first edit writes it into the server's containers.
  if (node && node.childCount > 0) {
    tr = tr.replaceWith(0, view.state.doc.content.size, node.content);
  }
  if (node) tr = withTopAttrs(tr, node);
  tr.setMeta(loroSyncPluginKey, { type: "update-state", state: { mapping, snapshot: null } });
  tr.setMeta("addToHistory", false);
  view.dispatch(tr);
}

/** Set the top node's attributes to `node`'s where they differ. */
function withTopAttrs(tr: Transaction, node: Node): Transaction {
  const current = tr.doc.attrs;
  for (const [name, value] of Object.entries(node.attrs)) {
    if (current[name] !== value) tr = tr.setDocAttribute(name, value);
  }
  return tr;
}

/** Drop cached nodes for every container an event touched, and their ancestors. */
function forgetChanged(doc: LoroDoc, event: { events: Array<{ target: ContainerID }> }, mapping: Mapping): void {
  for (const e of event.events) {
    let obj = doc.getContainerById(e.target) as { id: ContainerID; parent(): unknown } | undefined;
    while (obj) {
      mapping.delete(obj.id);
      obj = obj.parent() as typeof obj;
    }
  }
}

function applyRemote(
  view: EditorView,
  doc: LoroDoc,
  containerId: ContainerID,
  event: { events: Array<{ target: ContainerID }> },
  cursors: Cursors,
): void {
  if (view.isDestroyed) return;
  const state = loroSyncPluginKey.getState(view.state) as SyncState;
  // Read before the replace: positions in the document as it is now.
  const moved = domSelectionIfMoved(view);
  forgetChanged(doc, event, state.mapping);
  const root = field(doc, containerId);
  // Someone emptied the document: show the schema's empty document.
  const next = isEmpty(root)
    ? view.state.schema.topNodeType.createAndFill()!
    : (createNodeFromLoroObj(view.state.schema, root as never, state.mapping as never) as Node);
  const current = view.state.doc;
  const start = current.content.findDiffStart(next.content);
  let tr = view.state.tr;
  if (start != null) {
    let { a: endA, b: endB } = current.content.findDiffEnd(next.content)!;
    // A run of equal content at both ends can make the two ends cross.
    const overlap = start - Math.min(endA, endB);
    if (overlap > 0) {
      endA += overlap;
      endB += overlap;
    }
    tr = tr.replace(start, endA, next.slice(start, endB));
  }
  tr = withTopAttrs(tr, next);
  // Nothing this editor shows changed (an echo, a change elsewhere in the row).
  if (!tr.docChanged) return;
  // A node selection, a cell selection: ProseMirror maps it through the
  // replace and keeps its kind; a caret rebuilt from cursors would not.
  if (!(view.state.selection instanceof TextSelection)) {
    tr.setMeta(loroSyncPluginKey, { type: "non-local-updates" });
    tr.setMeta("addToHistory", false);
    view.dispatch(tr);
    return;
  }
  if (moved) {
    // The person moved the caret after the cursors were taken: map where it
    // is now through the replace.
    tr = tr.setSelection(
      TextSelection.between(tr.doc.resolve(tr.mapping.map(moved.anchor)), tr.doc.resolve(tr.mapping.map(moved.head))),
    );
    tr.setMeta(loroSyncPluginKey, { type: "non-local-updates" });
    tr.setMeta("addToHistory", false);
    view.dispatch(tr);
    return;
  }
  const anchor = cursors.anchor && cursorToPosition(cursors.anchor, doc, containerId, state.mapping);
  const head = cursors.head && cursorToPosition(cursors.head, doc, containerId, state.mapping);
  // Otherwise the selection stays where ProseMirror mapped it through the replace.
  if (anchor != null && anchor <= tr.doc.content.size) {
    const headPos = head != null && head <= tr.doc.content.size ? head : anchor;
    tr = tr.setSelection(TextSelection.between(tr.doc.resolve(anchor), tr.doc.resolve(headPos)));
  }
  tr.setMeta(loroSyncPluginKey, { type: "non-local-updates" });
  tr.setMeta("addToHistory", false);
  view.dispatch(tr);
}

/**
 * The selection as Loro cursors, bound to the character on the left of each
 * end. Null ends where the doc has no text to bind to.
 */
export function selectionToCursors(state: EditorState, doc: LoroDoc, containerId: ContainerID): Cursors {
  const { anchor, head } = state.selection;
  const a = positionToCursor(state.doc, anchor, doc, containerId);
  return { anchor: a, head: head === anchor ? a : positionToCursor(state.doc, head, doc, containerId) };
}

/**
 * The Loro container for each child of a node, in the binding's layout: a
 * run of text nodes is one LoroText, every other node one LoroMap.
 */
function childIndexFor(node: Node, pmIndex: number): number {
  let group = -1;
  let inText = false;
  for (let i = 0; i <= pmIndex && i < node.childCount; i++) {
    const child = node.child(i);
    if (child.isText) {
      if (!inText) group += 1;
      inText = true;
    } else {
      group += 1;
      inText = false;
    }
  }
  return group;
}

/**
 * A cursor for a document position. Walks the position's path from the
 * field's root down, so it does not depend on node identity (a replace
 * rebuilds the nodes along its path).
 *
 * Inside text the cursor binds to a character: by default the one on the
 * left, so text typed at a caret goes before it. `bind: "right"` binds to the
 * character on the right instead, for the start of a range, so text typed
 * just before the range stays out of it. Where there is no character on that
 * side, it binds to the other one.
 */
export function positionToCursor(
  root: Node,
  pos: number,
  doc: LoroDoc,
  containerId: ContainerID,
  options: { bind?: "left" | "right" } = {},
): Cursor | null {
  const bind = options.bind ?? "left";
  if (pos < 0 || pos > root.content.size) return null;
  const $pos = root.resolve(pos);
  let map = field(doc, containerId);
  for (let depth = 0; depth < $pos.depth; depth++) {
    const children = map.get(CHILDREN_KEY);
    if (!(children instanceof LoroList)) return null;
    const child = children.get(childIndexFor($pos.node(depth), $pos.index(depth)));
    if (!(child instanceof LoroMap)) return null;
    map = child;
  }
  const children = map.get(CHILDREN_KEY);
  if (!(children instanceof LoroList)) return null;
  if (children.length === 0) return children.getCursor(0) ?? null;
  // Between blocks (a node selection, a gap cursor): bind to the next child.
  if (!$pos.parent.inlineContent) {
    return children.getCursor(childIndexFor($pos.parent, $pos.index()) + ($pos.index() >= $pos.parent.childCount ? 1 : 0)) ?? null;
  }
  // Walk the textblock's children alongside the editor's: a run of text
  // nodes is one LoroText, every other inline node one child of its size.
  const parent = $pos.parent;
  const offset = $pos.parentOffset;
  let group = -1;
  let pmOffset = 0;
  let runStart = -1;
  for (let i = 0; i < parent.childCount; i++) {
    const child = parent.child(i);
    if (child.isText) {
      if (runStart < 0) {
        group += 1;
        runStart = pmOffset;
      }
    } else {
      // The caret at the end of a text run binds to that run, so typing
      // there continues it.
      if (runStart >= 0 && offset <= pmOffset) return textCursor(children.get(group), offset - runStart, bind);
      runStart = -1;
      group += 1;
      if (offset === pmOffset) return children.getCursor(group) ?? null;
    }
    pmOffset += child.nodeSize;
  }
  if (runStart >= 0 && offset <= pmOffset) return textCursor(children.get(group), offset - runStart, bind);
  return children.getCursor(children.length) ?? null;
}

/** True for the second half of a UTF-16 surrogate pair (an emoji's, say). */
function isLowSurrogate(text: string): boolean {
  const code = text.charCodeAt(0);
  return code >= 0xdc00 && code <= 0xdfff;
}

/**
 * A cursor `offset` UTF-16 units into a text run, bound to the character on
 * its left (side 1: just after it), the whole character when it is a
 * surrogate pair. At the start of the run there is none, so it binds to the
 * first character. With `bind` "right" it binds to the character on its
 * right (side -1: just before it) while there is one.
 */
function textCursor(text: unknown, offset: number, bind: "left" | "right"): Cursor | null {
  if (!(text instanceof LoroText)) return null;
  if (offset <= 0) return text.getCursor(0, 0) ?? null;
  if (bind === "right" && offset < text.length) {
    const at = isLowSurrogate(text.slice(offset, offset + 1)) ? offset - 1 : offset;
    return text.getCursor(at, -1) ?? null;
  }
  let start = Math.min(offset, text.length) - 1;
  if (start > 0 && isLowSurrogate(text.slice(start, start + 1))) start -= 1;
  return text.getCursor(start, 1) ?? null;
}

/** Size in the editor of a container's node, from the binding's mapping. */
function mappedSize(mapping: Mapping, id: ContainerID): number | null {
  const mapped = mapping.get(id);
  if (mapped == null) return null;
  return Array.isArray(mapped) ? mapped.reduce((n, node) => n + node.nodeSize, 0) : mapped.nodeSize;
}

/**
 * The document position a cursor points at now. Uses the mapping, which
 * createNodeFromLoroObj has just filled for the whole tree.
 */
export function cursorToPosition(
  cursor: Cursor,
  doc: LoroDoc,
  containerId: ContainerID,
  mapping: Mapping,
  options: { acceptMoved?: boolean } = {},
): number | null {
  const rootId = field(doc, containerId).id;
  const target = cursor.containerId();
  let index: number;
  let childId: ContainerID;
  let node: LoroMap | undefined;
  if (target.endsWith("List")) {
    const list = doc.getContainerById(target) as LoroList | undefined;
    const pos = doc.getCursorPos(cursor);
    const owner = list?.parent() as LoroMap | undefined;
    if (!list || !pos || (pos.update && !options.acceptMoved) || !owner) return null;
    // Inside a node with no text, or before an inline node: count the
    // children before it.
    index = 0;
    for (let i = 0; i < pos.offset; i++) {
      const child = list.get(i) as { id?: ContainerID } | undefined;
      const size = child?.id ? mappedSize(mapping, child.id) : null;
      if (size == null) return null;
      index += size;
    }
    if (owner.id === rootId) return index;
    childId = owner.id;
    node = (owner.parent() as LoroList | undefined)?.parent() as LoroMap | undefined;
    index += 1; // into the owner node
  } else {
    const text = doc.getContainerById(target) as LoroText | undefined;
    const pos = doc.getCursorPos(cursor);
    // `update` means the character was deleted (a split moves text into a
    // new paragraph as a delete and an insert). For this editor's own caret,
    // ProseMirror's mapping through the replace places it better; another
    // person's caret has no mapping, so it takes Loro's nearest position
    // (`acceptMoved`).
    if (!text || !pos || (pos.update && !options.acceptMoved)) return null;
    // The cursor names a character; side 1 means just after it, which is
    // two UTF-16 units for a surrogate pair.
    const width =
      pos.side === 1 ? (pos.offset + 1 < text.length && isLowSurrogate(text.slice(pos.offset + 1, pos.offset + 2)) ? 2 : 1) : 0;
    index = pos.offset + width;
    childId = text.id;
    node = (text.parent() as LoroList | undefined)?.parent() as LoroMap | undefined;
  }
  while (node) {
    const children = node.get(CHILDREN_KEY);
    if (!(children instanceof LoroList)) return null;
    for (let i = 0; i < children.length; i++) {
      const child = children.get(i) as { id?: ContainerID } | undefined;
      if (!child?.id || child.id === childId) break;
      const size = mappedSize(mapping, child.id);
      if (size == null) return null;
      index += size;
    }
    if (node.id === rootId) return index;
    childId = node.id;
    node = (node.parent() as LoroList | undefined)?.parent() as LoroMap | undefined;
    index += 1; // the opening of the node we came out of
  }
  return null;
}
