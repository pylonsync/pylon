/**
 * React binding for a ProseMirror (Tiptap) field.
 *
 *   const { doc, containerId } = useProseMirrorDoc("Doc", id, "body", schema);
 *   if (!containerId) return <Loading />;
 *   const plugins = [LoroSyncPlugin({ doc, containerId }), LoroUndoPlugin({ doc })];
 *
 * `containerId` is null until the server's snapshot brings the field's
 * container; mount the editor after that (see ./index.ts for why a client
 * never creates it). Edits the editor makes reach the server through the
 * row's outbox, like every edit on a `useLoroDoc` doc.
 */
import type { ContainerID, LoroDoc } from "loro-crdt";
import type { Schema } from "prosemirror-model";
import { useMemo } from "react";
import { useLoroDoc } from "../index";
import { configureProseMirrorMarks, findProseMirrorContainer } from "./index";

export function useProseMirrorDoc(
  entity: string,
  id: string,
  field: string,
  schema: Schema,
): { doc: LoroDoc; containerId: ContainerID | null } {
  const doc = useLoroDoc(entity, id);
  // Before any binding writes a mark. Idempotent, so once per doc is enough.
  useMemo(() => configureProseMirrorMarks(doc, schema), [doc, schema]);
  // useLoroDoc re-renders on every applied frame, so this picks up the
  // container as soon as the snapshot lands.
  return { doc, containerId: findProseMirrorContainer(doc, field) };
}
