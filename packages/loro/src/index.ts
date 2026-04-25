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
import { db } from "@pylonsync/react";
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

// Suppress unused-import error on `useEffect` — keeping the import in
// scope for the upcoming useLoroSubscribe(entity, id) variant that
// needs it.
void useEffect;
