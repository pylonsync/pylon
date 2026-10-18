/**
 * Other people's carets and selections in a ProseMirror editor bound with
 * ProseMirrorSyncPlugin.
 *
 * Each person's selection travels as two Loro cursors (encoded, base64),
 * which point at characters, not positions: a caret stays on its words while
 * anyone edits before it. Send them through the room tied to the row
 * (`useRoom({ entity, id })`), so only people who may read the document see
 * them:
 *
 *   const room = useRoom({ entity: "Doc", id }, me.id);
 *   const plugin = ProseMirrorCursorsPlugin({
 *     doc, containerId,
 *     onSelection: (selection) => room.setPresence({ name, color, selection }),
 *   });
 *   // whenever room.peers changes:
 *   setRemoteCursors(view, room.peers.map((p) => ({ id: p.id, ...p.presence })));
 */
import { type ContainerID, Cursor, type LoroDoc } from "loro-crdt";
import { loroSyncPluginKey } from "loro-prosemirror";
import type { Node } from "prosemirror-model";
import { type EditorState, Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet, type EditorView } from "prosemirror-view";
import { cursorToPosition, selectionToCursors } from "./sync";

/** A selection as encoded Loro cursors. Null where there is no text to bind to. */
export interface EncodedSelection {
  anchor: string | null;
  head: string | null;
}

/** Someone else in the document. */
export interface RemoteCursor {
  /** Stable per person (the room peer id). */
  id: string;
  name: string;
  /** A CSS color for the caret and the selection. */
  color: string;
  selection?: EncodedSelection | null;
}

export interface ProseMirrorCursorsPluginProps {
  doc: LoroDoc;
  containerId: ContainerID;
  /** Called when this person's selection moves, with its encoded cursors. */
  onSelection?: (selection: EncodedSelection) => void;
}

export const proseMirrorCursorsKey = new PluginKey<RemoteCursor[]>("pylonRemoteCursors");

/** A Loro cursor as text (base64), to store or send. */
export function encodeCursor(cursor: Cursor): string {
  return toBase64(cursor.encode());
}

/** A cursor from {@link encodeCursor}, or null when the text is not one. */
export function decodeCursor(text: string): Cursor | null {
  try {
    return Cursor.decode(fromBase64(text));
  } catch {
    return null;
  }
}

function toBase64(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}

function fromBase64(text: string): Uint8Array {
  const s = atob(text);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

/** This editor's selection as encoded Loro cursors. */
export function encodeSelection(state: EditorState, doc: LoroDoc, containerId: ContainerID): EncodedSelection {
  const { anchor, head } = selectionToCursors(state, doc, containerId);
  return {
    anchor: anchor ? toBase64(anchor.encode()) : null,
    head: head ? toBase64(head.encode()) : null,
  };
}

/** Show these people's carets and selections. Replaces the previous list. */
export function setRemoteCursors(view: EditorView, cursors: RemoteCursor[]): void {
  view.dispatch(view.state.tr.setMeta(proseMirrorCursorsKey, cursors).setMeta("addToHistory", false));
}

function caret(cursor: RemoteCursor): HTMLElement {
  const el = document.createElement("span");
  el.className = "pylon-remote-caret";
  el.style.borderLeft = `2px solid ${cursor.color}`;
  el.style.marginLeft = "-1px";
  el.style.marginRight = "-1px";
  el.style.position = "relative";
  el.setAttribute("aria-hidden", "true");
  const label = document.createElement("span");
  label.className = "pylon-remote-caret-label";
  label.textContent = cursor.name;
  label.style.position = "absolute";
  label.style.left = "-2px";
  label.style.bottom = "100%";
  label.style.whiteSpace = "nowrap";
  label.style.background = cursor.color;
  label.style.color = "white";
  label.style.fontSize = "11px";
  label.style.lineHeight = "1.4";
  label.style.padding = "0 4px";
  label.style.borderRadius = "3px 3px 3px 0";
  label.style.pointerEvents = "none";
  el.appendChild(label);
  return el;
}

function resolve(encoded: string | null | undefined, doc: LoroDoc, containerId: ContainerID, state: EditorState): number | null {
  if (!encoded) return null;
  const sync = loroSyncPluginKey.getState(state) as { mapping?: Map<ContainerID, Node | Node[]> } | undefined;
  if (!sync?.mapping) return null;
  try {
    // A deleted character's cursor moves to Loro's nearest position, so the
    // caret stays visible until its owner sends a new selection.
    const pos = cursorToPosition(Cursor.decode(fromBase64(encoded)), doc, containerId, sync.mapping, {
      acceptMoved: true,
    });
    return pos != null && pos >= 0 && pos <= state.doc.content.size ? pos : null;
  } catch {
    return null;
  }
}

function decorations(state: EditorState, cursors: RemoteCursor[], doc: LoroDoc, containerId: ContainerID): DecorationSet {
  const decos: Decoration[] = [];
  for (const cursor of cursors) {
    const head = resolve(cursor.selection?.head, doc, containerId, state);
    if (head == null) continue;
    const anchor = resolve(cursor.selection?.anchor, doc, containerId, state) ?? head;
    if (anchor !== head) {
      decos.push(
        Decoration.inline(Math.min(anchor, head), Math.max(anchor, head), {
          style: `background-color: color-mix(in srgb, ${cursor.color} 22%, transparent)`,
          class: "pylon-remote-selection",
        }),
      );
    }
    decos.push(Decoration.widget(head, () => caret(cursor), { key: `caret-${cursor.id}-${cursor.color}-${cursor.name}`, side: -1 }));
  }
  return DecorationSet.create(state.doc, decos);
}

export function ProseMirrorCursorsPlugin({ doc, containerId, onSelection }: ProseMirrorCursorsPluginProps): Plugin {
  // Decorations are recomputed for each new document or peer list.
  let cache: { doc: Node; cursors: RemoteCursor[]; set: DecorationSet } | null = null;
  return new Plugin<RemoteCursor[]>({
    key: proseMirrorCursorsKey,
    state: {
      init: () => [],
      apply: (tr, value) => (tr.getMeta(proseMirrorCursorsKey) as RemoteCursor[] | undefined) ?? value,
    },
    props: {
      decorations(state) {
        const cursors = proseMirrorCursorsKey.getState(state) ?? [];
        if (cursors.length === 0) return DecorationSet.empty;
        if (cache && cache.doc === state.doc && cache.cursors === cursors) return cache.set;
        const set = decorations(state, cursors, doc, containerId);
        cache = { doc: state.doc, cursors, set };
        return set;
      },
    },
    view: () => {
      let last = "";
      return {
        update(view) {
          if (!onSelection || !view.hasFocus()) return;
          const encoded = encodeSelection(view.state, doc, containerId);
          const key = `${encoded.anchor}|${encoded.head}`;
          if (key === last) return;
          last = key;
          onSelection(encoded);
        },
      };
    },
  });
}
