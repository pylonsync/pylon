/**
 * ProseMirror (Tiptap) documents in a Pylon row.
 *
 * A field declared `field.richtext().crdt("prosemirror")` keeps its
 * document under its key of the row's `"row"` map, in the layout the
 * `loro-prosemirror` binding writes. The server reads that tree and
 * projects it to the row as ProseMirror JSON (crates/crdt).
 *
 * The server creates the field's container when the row is inserted.
 * Clients never create it: `useLoroDoc` returns an empty doc before the
 * server's snapshot arrives, and a container made in it would lose the
 * key to the server's, leaving the editor bound to a container nothing
 * reads. Wait for the server's instead, then bind:
 *
 *   const doc = useLoroDoc("Doc", id);
 *   configureProseMirrorMarks(doc, schema);
 *   const containerId = await waitForProseMirrorContainer(doc, "body");
 *   const plugin = ProseMirrorSyncPlugin({ doc, containerId }); // ./sync
 *
 * In a server function, read and change the document without an editor
 * with `readProseMirror` and `writeProseMirror`.
 *
 * A mark type holds one value per character: two marks of one type
 * cannot overlap. Keep overlapping anchors (comment threads) as Loro
 * cursors next to the document and draw them as decorations.
 */
import type { ContainerID, LoroDoc, LoroMap } from "loro-crdt";
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
 * The container of `field`'s document, or null when the doc does not hold
 * it yet (the server's snapshot has not arrived). Never creates it.
 */
export function findProseMirrorContainer(doc: LoroDoc, field: string): ContainerID | null {
  return fieldMap(doc, field)?.id ?? null;
}

/**
 * The container of `field`'s document, once the doc holds it: at once when
 * it is there, else when an import or edit brings it. Pass `signal` to stop
 * waiting (the promise then rejects with the signal's reason).
 */
export function waitForProseMirrorContainer(
  doc: LoroDoc,
  field: string,
  options: { signal?: AbortSignal } = {},
): Promise<ContainerID> {
  const found = findProseMirrorContainer(doc, field);
  if (found) return Promise.resolve(found);
  return new Promise((resolve, reject) => {
    const { signal } = options;
    const stop = doc.subscribe(() => {
      const id = findProseMirrorContainer(doc, field);
      if (!id) return;
      stop();
      signal?.removeEventListener("abort", onAbort);
      resolve(id);
    });
    function onAbort() {
      stop();
      reject(signal?.reason ?? new Error("stopped waiting for the document"));
    }
    if (signal?.aborted) onAbort();
    else signal?.addEventListener("abort", onAbort, { once: true });
  });
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
 * schema's marks first. Throws when the doc has no container for the field
 * (a row inserted by a server older than the field, or a doc that has not
 * loaded); it never creates one.
 */
export function writeProseMirror(doc: LoroDoc, field: string, schema: Schema, node: Node): void {
  const containerId = findProseMirrorContainer(doc, field);
  if (!containerId) {
    throw new Error(`The doc has no container for ${field}; the server creates it when the row is inserted.`);
  }
  configureProseMirrorMarks(doc, schema);
  updateLoroToPmState(doc as never, new Map(), EditorState.create({ schema, doc: node }), containerId);
}
