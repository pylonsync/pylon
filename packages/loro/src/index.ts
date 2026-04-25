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

import { useSyncExternalStore, useEffect } from "react";
import type { LoroDoc, LoroText, LoroMap } from "loro-crdt";
import { db, getBaseUrl, getReactStorage, storageKey } from "@pylonsync/react";
import type { SyncEngine } from "@pylonsync/sync";
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

function ensureAttached(): void {
  const sync = db.sync;
  if (attachedSync === sync) return;

  // Tear down the previous engine's handler if init() swapped it
  // (test harness or re-init at runtime).
  if (unsubscribeBinaryHandler) {
    unsubscribeBinaryHandler();
    unsubscribeBinaryHandler = null;
  }
  unsubscribeBinaryHandler = sync.onBinaryFrame((bytes: Uint8Array) => {
    globalRegistry.applyBinaryFrame(bytes);
  });
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

  // useSyncExternalStore drives re-renders. The snapshot is the doc
  // itself (referentially stable across calls — same instance from
  // the registry), so React's bail-out keeps re-renders bounded to
  // when the registry's listener actually fires.
  const subscribe = (notify: () => void) =>
    globalRegistry.subscribe(entity, id, notify);
  const getSnapshot = () => globalRegistry.doc(entity, id);
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

/**
 * Convenience: get a `LoroText` container at the given top-level key,
 * creating it if absent. Wraps `doc.getText(key)` so callers don't
 * have to import loro-crdt directly for the common case.
 */
export function getLoroText(doc: LoroDoc, key: string): LoroText {
  return doc.getText(key);
}

/**
 * Convenience: get a `LoroMap` container at the given top-level key.
 */
export function getLoroMap(doc: LoroDoc, key: string): LoroMap {
  return doc.getMap(key);
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
  const text = doc.getText(field);
  const value = text.toString();
  const setValue = (next: string): void => {
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

    const update = doc.export({ mode: "update", from: beforeVv });
    if (update.length === 0) {
      return; // No-op (e.g. setValue called with the same value)
    }
    void pushCrdtUpdate(entity, id, update);
  };
  return [value, setValue];
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
  const baseUrl = getBaseUrl();
  const token = getReactStorage().get(storageKey("token"));
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (token) headers["Authorization"] = `Bearer ${token}`;
  try {
    await fetch(
      `${baseUrl}/api/crdt/${encodeURIComponent(entity)}/${encodeURIComponent(id)}`,
      {
        method: "POST",
        headers,
        body: JSON.stringify({ update: bytesToHex(update) }),
      },
    );
    // Server acknowledges with {ok: true}. The merged-state broadcast
    // arrives over the WS, applied automatically by the registry; no
    // need to read the response body here.
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
  attachedSync = null;
}

