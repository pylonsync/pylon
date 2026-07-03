// ---------------------------------------------------------------------------
// @pylonsync/loro
//
// Local-first React layer on top of Pylon's CRDT broadcast.
//
//   import { useLoroDoc, getLoroText } from "@pylonsync/loro";
//
//   function MessageBody({ messageId }: { messageId: string }) {
//     const doc = useLoroDoc("Message", messageId);
//     const text = getLoroText(doc, "body").toString();
//     return <p>{text}</p>;
//   }
//
// The hook subscribes to binary CRDT frames the server broadcasts on
// every CRDT-mode write. Two browser tabs editing the same row
// converge through Loro's CRDT merge — concurrent same-field writes
// don't lose data the way LWW would.
// ---------------------------------------------------------------------------

import { useSyncExternalStore, useEffect, useRef } from "react";
import type { FormEvent, RefObject } from "react";
import type { LoroDoc, LoroText, LoroMap } from "loro-crdt";
import {
  LoroText as LoroTextCtor,
  LoroMap as LoroMapCtor,
} from "loro-crdt";
import { db, getBaseUrl, getReactStorage, storageKey } from "@pylonsync/react";
import { pylonFetch, type SyncEngine } from "@pylonsync/sync";
import { globalRegistry, LoroRegistry } from "./registry";

export { LoroRegistry, globalRegistry } from "./registry";
export {
  decodeCrdtFrame,
  encodeCrdtFrame,
  CRDT_FRAME_SNAPSHOT,
  CRDT_FRAME_UPDATE,
} from "./wire";
export type { CrdtFrame } from "./wire";

// ---------------------------------------------------------------------------
// Sync engine ↔ registry wiring
//
// Connect once, lazily — the first useLoroDoc call sets up the binary
// handler. Subsequent calls reuse the same registration. Re-registers
// transparently on hot module reload (the registry's Set semantics
// dedup the handler).
// ---------------------------------------------------------------------------

let attachedSync: SyncEngine | null = null;
let unsubscribeBinaryHandler: (() => void) | null = null;
let unsubscribeRowEviction: (() => void) | null = null;

function ensureAttached(): void {
  const sync = db.sync;
  if (attachedSync === sync) return;

  // Tear down the previous engine's handlers if init() swapped it
  // (test harness or re-init at runtime).
  if (unsubscribeBinaryHandler) {
    unsubscribeBinaryHandler();
    unsubscribeBinaryHandler = null;
  }
  if (unsubscribeRowEviction) {
    unsubscribeRowEviction();
    unsubscribeRowEviction = null;
  }
  unsubscribeBinaryHandler = sync.onBinaryFrame((bytes: Uint8Array) => {
    globalRegistry.applyBinaryFrame(bytes);
  });
  // The server's `row-revoked` envelope signals that the subscriber's
  // policy was revoked mid-session for a specific row. Drop the
  // cached LoroDoc so the collaborative handle unmounts instead of
  // lingering with stale state until the tab closes.
  unsubscribeRowEviction = sync.addRowEvictionListener(
    (entity: string, rowId: string) => {
      globalRegistry.evict(entity, rowId);
    },
  );
  attachedSync = sync;
}

// ---------------------------------------------------------------------------
// React hooks
// ---------------------------------------------------------------------------

/**
 * Subscribe to the LoroDoc for a row. Returns the same doc instance
 * across renders for the same `(entity, id)` pair. The doc updates
 * in place as binary CRDT frames arrive from the server; React
 * re-renders the calling component on every applied frame via
 * `useSyncExternalStore`.
 *
 * The doc is the *source of truth* for CRDT-mode entities — local
 * mutations should go through doc operations (`getText().insert(...)`,
 * `getMap().set(...)`) rather than `db.update`. The server's
 * SQLite-projected row catches up via the next binary frame.
 *
 * For `crdt: false` entities the LoroDoc stays empty (the server
 * never broadcasts a frame for them). Use `db.useQueryOne(entity, id)`
 * for those instead.
 */
export function useLoroDoc(entity: string, id: string): LoroDoc {
  ensureAttached();

  // Tell the server we want binary CRDT frames for this row. Refcounted
  // inside the sync engine, so two components watching the same row
  // don't fight over the subscription. Without this the server never
  // sends a binary frame and the LoroDoc stays empty forever — the
  // notifier filters by subscriber set rather than fanning out to
  // every WS client.
  //
  // We use `useEffect` (not `useSyncExternalStore`'s subscribe) because
  // the subscribe call is a side effect on the network, not a React
  // store subscription. The store subscription stays registry-local
  // and fires on every applied frame.
  useEffect(() => {
    const sync = db.sync;
    sync.subscribeCrdt(entity, id);
    return () => {
      sync.unsubscribeCrdt(entity, id);
    };
  }, [entity, id]);

  // useSyncExternalStore drives re-renders. The SNAPSHOT is the row's
  // monotonic version counter, not the doc: the doc instance is
  // intentionally stable across renders, and React bails out of
  // re-rendering when the snapshot is Object.is-equal — a stable doc
  // snapshot would swallow every notification. The version bumps on
  // each applied frame AND on each local `touch()` after a commit, so
  // consumers re-render optimistically on their own edits instead of
  // waiting for the server's post-merge echo.
  const subscribe = (notify: () => void) =>
    globalRegistry.subscribe(entity, id, notify);
  const getVersion = () => globalRegistry.version(entity, id);
  useSyncExternalStore(subscribe, getVersion, getVersion);
  return globalRegistry.doc(entity, id);
}

/**
 * The server's projection contract (crates/crdt): each entity row's doc
 * holds ONE root `LoroMap` named "row"; its keys are the entity's field
 * names, and a `crdt: "text"` field is a `LoroText` CHILD of that map.
 * The server seeds those containers at insert and projects them back to
 * SQL columns after every merge — a text container anywhere else in the
 * doc is invisible to it. Every field accessor here must resolve through
 * this map or client edits silently diverge from the server's view.
 */
const ROOT_MAP = "row";

function containerOfKind<C>(value: unknown, kind: string): C | null {
  if (
    value &&
    typeof value === "object" &&
    typeof (value as { kind?: unknown }).kind === "function" &&
    (value as { kind: () => string }).kind() === kind
  ) {
    return value as C;
  }
  return null;
}

/**
 * Read-only resolve of the field's `LoroText` under the `"row"` root
 * map. Returns null when the container doesn't exist yet — crucially it
 * NEVER creates one: a read-path create races the server's catch-up
 * snapshot, and when the server's seeded container wins the map-key LWW
 * the eagerly-created one is orphaned — every edit made through a
 * captured handle then mutates a container nothing projects or renders.
 */
export function resolveLoroText(doc: LoroDoc, key: string): LoroText | null {
  return containerOfKind<LoroText>(doc.getMap(ROOT_MAP).get(key), "Text");
}

/**
 * Get the `LoroText` for an entity FIELD, resolving through the server's
 * `"row"` root map. The server seeds the container at insert, so the
 * common case finds it (carrying the row's initial value); creation only
 * happens on the WRITE path for rows that predate CRDT mode. Callers
 * must re-resolve per operation rather than capture the handle — after a
 * catch-up snapshot merges, the live container can be a different
 * instance than the one an earlier render saw. Identified by `kind()`
 * rather than instanceof so a duplicated loro-crdt module in the bundle
 * can't break the check.
 */
export function getLoroText(doc: LoroDoc, key: string): LoroText {
  const existing = resolveLoroText(doc, key);
  if (existing) return existing;
  return doc
    .getMap(ROOT_MAP)
    .setContainer(key, new LoroTextCtor()) as LoroText;
}

/**
 * Get the `LoroMap` for an entity FIELD (a map child of the `"row"`
 * root). For the root map itself use `doc.getMap("row")` directly.
 */
export function getLoroMap(doc: LoroDoc, key: string): LoroMap {
  const row = doc.getMap(ROOT_MAP);
  const existing = containerOfKind<LoroMap>(row.get(key), "Map");
  if (existing) return existing;
  return row.setContainer(key, new LoroMapCtor()) as LoroMap;
}

/**
 * High-level hook for the most common CRDT case: a single text field
 * shared across clients. Returns `[value, setValue]` matching React's
 * useState shape so existing controlled-input components drop in.
 *
 * Two tabs editing the same `(entity, id, field)` converge through
 * Loro's text CRDT — concurrent same-position writes interleave
 * deterministically rather than one stomping the other.
 *
 * `setValue` performs a whole-text replace on every call. Loro's
 * text-CRDT update path treats this as a delete-then-insert against
 * the prior version vector, so concurrent edits to disjoint regions
 * still merge correctly. After applying locally, the hook ships the
 * incremental update to the server (POST /api/crdt/<entity>/<id>),
 * which re-projects to SQLite and broadcasts the merged snapshot
 * back to every connected tab. A future variant could expose lower-
 * level insert/delete ops for IME-friendly diff-aware editing.
 *
 * For boring CRUD use `db.useQueryOne` instead — this hook only
 * lights up for entities marked `crdt: true` (the default) AND
 * fields with `crdt: "text"` (or `richtext` type, which defaults
 * to LoroText).
 */
export function useCollabText(
  entity: string,
  id: string,
  field: string,
): [string, (next: string) => void] {
  const doc = useLoroDoc(entity, id);
  // Re-resolved every render/operation, never captured: the live
  // container can change identity when the catch-up snapshot merges.
  const value = resolveLoroText(doc, field)?.toString() ?? "";
  const setValue = (next: string): void => {
    const text = getLoroText(doc, field);
    // Capture the version vector BEFORE the mutation so we can ship
    // exactly the new ops to the server (incremental delta, not the
    // whole snapshot). Loro's `export({mode: "update", from: vv})`
    // returns the bytes the server hasn't seen.
    const beforeVv = doc.oplogVersion();
    const len = text.length;
    if (len > 0) {
      text.delete(0, len);
    }
    if (next.length > 0) {
      text.insert(0, next);
    }
    doc.commit();
    // Optimistic render: notify every consumer of this row NOW.
    globalRegistry.touch(entity, id);

    const update = doc.export({ mode: "update", from: beforeVv });
    if (update.length === 0) {
      return; // No-op (e.g. setValue called with the same value)
    }
    void pushCrdtUpdate(entity, id, update);
  };
  return [value, setValue];
}

/**
 * Diff-aware collaborative binding for a `<textarea>` / `<input>` — the
 * editor-grade counterpart to {@link useCollabText}.
 *
 * `useCollabText`'s `setValue` deletes the whole field and re-inserts it on
 * every keystroke, which converges but moves the caret — fine for a one-line
 * topic, jarring in a document editor. This hook instead computes the MINIMAL
 * single splice between the old and new value (shared prefix + shared suffix)
 * and applies just that `insert`/`delete` to the `LoroText`, shipping only the
 * incremental delta. Remote frames are merged back into the element with the
 * local caret remapped across the change, so two people typing in the same
 * document don't fight over the cursor.
 *
 * LoroText indices are UTF-16 (matching JS string offsets and
 * `selectionStart`/`selectionEnd`), so the prefix/suffix diff computed on the
 * raw element value lines up with Loro exactly — no codepoint conversion, and
 * astral characters (emoji) splice correctly.
 *
 * Spread the returned `ref` + `onInput` onto the element, and use
 * `defaultValue` (NOT `value`) so the element stays uncontrolled and React
 * re-renders never clobber the caret. `value` is the live text, handy for a
 * side-by-side preview:
 *
 * ```tsx
 * const { ref, value, onInput } = useCollabTextarea("Document", id, "content");
 * return (
 *   <>
 *     <textarea ref={ref} defaultValue={value} onInput={onInput} />
 *     <MarkdownPreview source={value} />
 *   </>
 * );
 * ```
 */
export function useCollabTextarea<
  T extends HTMLTextAreaElement | HTMLInputElement = HTMLTextAreaElement,
>(
  entity: string,
  id: string,
  field: string,
): {
  ref: RefObject<T | null>;
  value: string;
  onInput: (e: FormEvent<T>) => void;
} {
  const doc = useLoroDoc(entity, id);
  // Re-resolved every render, never captured: the live container can
  // change identity when the catch-up snapshot merges (see
  // resolveLoroText), and a stale handle would edit an orphan.
  const value = resolveLoroText(doc, field)?.toString() ?? ""; // re-renders on every applied frame
  const ref = useRef<T | null>(null);
  // The value we last reconciled into the DOM element. Lets us tell our own
  // edits (element already correct — skip) from remote frames (patch the DOM).
  const domValue = useRef<string>(value);

  // Apply REMOTE changes to the element, preserving the caret. Runs after each
  // render whose doc text differs from what the element currently shows.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (el.value === value) {
      domValue.current = value;
      return;
    }
    const prev = el.value;
    const selStart = el.selectionStart ?? prev.length;
    const selEnd = el.selectionEnd ?? prev.length;
    el.value = value;
    try {
      el.setSelectionRange(
        remapCaret(prev, value, selStart),
        remapCaret(prev, value, selEnd),
      );
    } catch {
      // Some element types don't support setSelectionRange — ignore.
    }
    domValue.current = value;
  }, [value]);

  const onInput = (e: FormEvent<T>): void => {
    const next = (e.target as T).value;
    const prev = domValue.current;
    if (next === prev) return;
    const { index, deleteCount, insert } = spliceDiff(prev, next);
    // Resolve the container per keystroke (see resolveLoroText) and
    // capture the version vector BEFORE mutating so we ship exactly the
    // new ops.
    const text = getLoroText(doc, field);
    const beforeVv = doc.oplogVersion();
    if (deleteCount > 0) text.delete(index, deleteCount);
    if (insert.length > 0) text.insert(index, insert);
    doc.commit();
    domValue.current = next;
    // Optimistic render: the preview (and any other consumer of this
    // row) updates on the local commit, not on the server echo.
    globalRegistry.touch(entity, id);
    const update = doc.export({ mode: "update", from: beforeVv });
    if (update.length > 0) void pushCrdtUpdate(entity, id, update);
  };

  return { ref, value, onInput };
}

/**
 * The minimal single splice that turns `prev` into `next`: shared prefix of
 * length `index`, shared suffix, with the differing middle of `prev`
 * (`deleteCount` chars) replaced by `insert`. UTF-16 units throughout to match
 * LoroText.
 */
function spliceDiff(
  prev: string,
  next: string,
): { index: number; deleteCount: number; insert: string } {
  const max = Math.min(prev.length, next.length);
  let p = 0;
  while (p < max && prev.charCodeAt(p) === next.charCodeAt(p)) p++;
  let s = 0;
  while (
    s < max - p &&
    prev.charCodeAt(prev.length - 1 - s) === next.charCodeAt(next.length - 1 - s)
  ) {
    s++;
  }
  return {
    index: p,
    deleteCount: prev.length - p - s,
    insert: next.slice(p, next.length - s),
  };
}

/**
 * Map a caret offset in `prev` to the equivalent offset in `next` across the
 * single splice between them. Before the change → unchanged; after → shifted
 * by the length delta; inside the replaced region → clamped to the splice end.
 */
function remapCaret(prev: string, next: string, caret: number): number {
  const { index, deleteCount, insert } = spliceDiff(prev, next);
  if (caret <= index) return caret;
  if (caret >= index + deleteCount) {
    return caret + (insert.length - deleteCount);
  }
  return index + insert.length;
}

// ---------------------------------------------------------------------------
// Upstream push — POST /api/crdt/<entity>/<row_id>
//
// Wraps the binary Loro update in a JSON envelope ({update: hex}) so it
// flows through Pylon's existing UTF-8-only HTTP body channel. Hex
// (vs base64) keeps the encoder zero-dep on both sides; bandwidth
// overhead is 2x, fine for the typical sub-1KB CRDT delta.
// ---------------------------------------------------------------------------

async function pushCrdtUpdate(
  entity: string,
  id: string,
  update: Uint8Array,
): Promise<void> {
  try {
    // Server acknowledges with {ok: true}. The merged-state broadcast
    // arrives over the WS, applied automatically by the registry — no
    // need to read the response body here, but pylonFetch surfaces
    // structured errors so failed pushes log instead of vanishing.
    await pylonFetch(
      {
        baseUrl: getBaseUrl(),
        getToken: () =>
          (getReactStorage().get(storageKey("token")) ?? undefined) as
            | string
            | undefined,
      },
      `/api/crdt/${encodeURIComponent(entity)}/${encodeURIComponent(id)}`,
      {
        method: "POST",
        json: { update: bytesToHex(update) },
      },
    );
  } catch (err) {
    console.warn(`[loro] CRDT push failed for ${entity}/${id}:`, err);
  }
}

function bytesToHex(bytes: Uint8Array): string {
  const out = new Array<string>(bytes.length);
  for (let i = 0; i < bytes.length; i++) {
    out[i] = bytes[i].toString(16).padStart(2, "0");
  }
  return out.join("");
}

/**
 * Manual teardown. Tests use this to drop the binary handler when
 * spinning up multiple engines in sequence; production apps don't
 * need to call it.
 */
export function detachLoro(): void {
  if (unsubscribeBinaryHandler) {
    unsubscribeBinaryHandler();
    unsubscribeBinaryHandler = null;
  }
  if (unsubscribeRowEviction) {
    unsubscribeRowEviction();
    unsubscribeRowEviction = null;
  }
  attachedSync = null;
}

