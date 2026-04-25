// ---------------------------------------------------------------------------
// LoroRegistry — per-row LoroDoc cache + binary-frame router
//
// One process-wide registry. The SyncEngine's binary handler routes
// every incoming CRDT frame here; this class:
//
//   1. Decodes the frame
//   2. Looks up (or creates) the LoroDoc for (entity, row_id)
//   3. Imports the snapshot/update into the doc
//   4. Notifies subscribers so React re-renders
//
// Architectural note: the server-side broadcast is currently
// "send every CRDT update to every connected client" (see
// docs/RUNTIME.md / loro_store.rs). The registry honors the same
// shape — it doesn't filter incoming frames by interest. Apps that
// don't subscribe to a row still create+update its LoroDoc as
// frames arrive; harmless extra work, kept here so that when the
// server eventually adds per-client subscriptions the client side
// is ready to consume them.
// ---------------------------------------------------------------------------

import { LoroDoc } from "loro-crdt";
import {
  decodeCrdtFrame,
  CRDT_FRAME_SNAPSHOT,
  CRDT_FRAME_UPDATE,
} from "./wire";

type Listener = () => void;

interface DocEntry {
  doc: LoroDoc;
  listeners: Set<Listener>;
}

export class LoroRegistry {
  /** (entity, row_id) → cached LoroDoc + subscribers. Map key is
   *  joined `entity:row_id` to keep the hash one-dimensional. */
  private docs: Map<string, DocEntry> = new Map();

  /** Get-or-create the doc for a row. The returned doc is the same
   *  instance across calls so subscribers and consumers all see the
   *  same CRDT state. Loro's per-doc peer_id is generated on first
   *  construction; re-fetching the same row never produces a new
   *  peer_id (which would fragment the merge graph). */
  doc(entity: string, rowId: string): LoroDoc {
    return this.entry(entity, rowId).doc;
  }

  /** Subscribe to changes on a row's doc. Returns an unsubscribe fn.
   *  Calls the listener after every applied frame (snapshot or
   *  update) and after every local mutation that triggers Loro's
   *  internal subscribe events. */
  subscribe(entity: string, rowId: string, listener: Listener): () => void {
    const entry = this.entry(entity, rowId);
    entry.listeners.add(listener);
    return () => {
      entry.listeners.delete(listener);
      // Don't drop the doc when listeners hit 0 — a future hook
      // remount on the same row should see the same state. Eviction
      // policy comes alongside the per-row subscribe protocol.
    };
  }

  /** Apply a binary frame from the WebSocket. Decodes then routes to
   *  the matching doc; notifies subscribers. Returns true when the
   *  frame parsed and an entry was updated, false on decode failure
   *  or unknown frame type. */
  applyBinaryFrame(bytes: Uint8Array): boolean {
    const frame = decodeCrdtFrame(bytes);
    if (!frame) return false;
    if (frame.type !== CRDT_FRAME_SNAPSHOT && frame.type !== CRDT_FRAME_UPDATE) {
      return false;
    }
    const entry = this.entry(frame.entity, frame.rowId);
    try {
      entry.doc.import(frame.payload);
    } catch (err) {
      console.warn(
        `[loro] import failed for ${frame.entity}/${frame.rowId}:`,
        err,
      );
      return false;
    }
    for (const listener of entry.listeners) {
      try {
        listener();
      } catch (err) {
        console.warn("[loro] listener threw:", err);
      }
    }
    return true;
  }

  /** Drop the cached doc for a row. Tests + the eventual eviction
   *  policy. Subscribers receive a final notify before the entry
   *  is removed so they can detect the drop and re-create their
   *  view if needed. */
  evict(entity: string, rowId: string): void {
    const key = this.key(entity, rowId);
    const entry = this.docs.get(key);
    if (!entry) return;
    for (const listener of entry.listeners) {
      try {
        listener();
      } catch {
        /* swallow — we're tearing down anyway */
      }
    }
    this.docs.delete(key);
  }

  /** Number of cached docs. Diagnostic. */
  cachedRows(): number {
    return this.docs.size;
  }

  private entry(entity: string, rowId: string): DocEntry {
    const key = this.key(entity, rowId);
    let entry = this.docs.get(key);
    if (!entry) {
      entry = { doc: new LoroDoc(), listeners: new Set() };
      this.docs.set(key, entry);
    }
    return entry;
  }

  private key(entity: string, rowId: string): string {
    return `${entity}:${rowId}`;
  }
}

/** Process-wide singleton. The React hook reaches for this rather than
 *  threading the registry through context — every Pylon app is
 *  single-tenant per process, so a global registry is the simpler
 *  shape and matches how `db.useQuery` (singleton SyncEngine) already
 *  works. */
export const globalRegistry = new LoroRegistry();
