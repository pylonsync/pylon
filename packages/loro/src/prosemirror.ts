/**
 * ProseMirror (Tiptap) documents in a Pylon row.
 *
 * A field declared `field.richtext().crdt("prosemirror")` keeps its
 * document under its key of the row's `"row"` map, in the layout the
 * `loro-prosemirror` binding writes. The server reads that tree and
 * projects it to the row as ProseMirror JSON (crates/crdt).
 *
 * In the browser, bind an editor with the binding's `LoroSyncPlugin`:
 *
 *   const doc = useLoroDoc("Doc", id);
 *   configureProseMirrorMarks(doc, schema);
 *   const plugin = LoroSyncPlugin({ doc, containerId: proseMirrorContainerId(doc, "body") });
 *
 * In a server function, read and change the document without an editor
 * with `readProseMirror` and `writeProseMirror`.
 *
 * A mark type holds one value per character: two marks of one type
 * cannot overlap. Keep overlapping anchors (comment threads) as Loro
 * cursors next to the document and draw them as decorations.
 */
import { type ContainerID, type LoroDoc, LoroMap } from "loro-crdt";
import { createNodeFromLoroObj, updateLoroToPmState } from "loro-prosemirror";
import type { Node, Schema } from "prosemirror-model";
import { EditorState } from "prosemirror-state";

/** The root map every Pylon row doc keeps its fields in (crates/crdt). */
const ROOT_MAP = "row";

function fieldMap(doc: LoroDoc, field: string): LoroMap | null {
  const value = doc.getMap(ROOT_MAP).get(field) as { kind?: () => string } | undefined;
  return value && typeof value.kind === "function" && value.kind() === "Map"
    ? (value as unknown as LoroMap)
    : null;
}

/**
 * The container the editor binds to for `field`, for `LoroSyncPlugin`'s
 * `containerId`. Creates it when the row has none yet; insert rows with
 * an initial document so the server creates it instead, since two
 * peers creating it at once leave one copy orphaned.
 */
export function proseMirrorContainerId(doc: LoroDoc, field: string): ContainerID {
  const existing = fieldMap(doc, field);
  if (existing) return existing.id;
  const created = doc.getMap(ROOT_MAP).setContainer(field, new LoroMap());
  doc.commit();
  return created.id;
}

/**
 * Tell this copy of the doc how each mark of `schema` behaves at its
 * edges, as the binding expects: a mark that is `inclusive` (the default)
 * extends to text typed right after it. Every copy that writes marks
 * needs this; Loro refuses a mark it has no style for.
 */
export function configureProseMirrorMarks(doc: LoroDoc, schema: Schema): void {
  const styles: Record<string, { expand: "after" | "none" }> = {};
  for (const [name, type] of Object.entries(schema.marks)) {
    styles[name] = { expand: type.spec.inclusive !== false ? "after" : "none" };
  }
  doc.configTextStyle(styles);
}

/** The document in `field` as a ProseMirror node, or null when there is none. */
export function readProseMirror(doc: LoroDoc, field: string, schema: Schema): Node | null {
  const map = fieldMap(doc, field);
  if (!map || map.get("nodeName") == null) return null;
  return createNodeFromLoroObj(schema, map as never, new Map()) as Node;
}

/**
 * Write `node` into `field` as a diff against what the doc holds, so edits
 * other peers make at the same time still merge. Commits. Configures the
 * schema's marks first.
 */
export function writeProseMirror(doc: LoroDoc, field: string, schema: Schema, node: Node): void {
  configureProseMirrorMarks(doc, schema);
  const containerId = proseMirrorContainerId(doc, field);
  updateLoroToPmState(doc as never, new Map(), EditorState.create({ schema, doc: node }), containerId);
}
