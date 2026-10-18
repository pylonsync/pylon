// ---------------------------------------------------------------------------
// CrdtOutbox — sends a doc's local edits to the server
//
// Every edit made on this copy of a row's doc (by an editor binding, a
// hook, or app code) has to reach `POST /api/crdt/<entity>/<id>`. The
// outbox listens for local commits and sends the edits the server has not
// confirmed:
//
//   - One request at a time, so edits arrive in order. Commits made while
//     a request is in flight go out together in the next one.
//   - What it sends is every op of this peer after the last confirmed one,
//     so a failed request loses nothing: the next send carries it again.
//     Ops of other peers are never sent back; they came from the server.
//   - A network error or a 5xx, 408 or 429 retries with backoff. Any other
//     4xx (policy denied, row gone, bad update) stops the outbox: sending
//     the same edits again would fail the same way.
//
// The server merges idempotently, so an edit sent twice (a response lost
// after the server applied it) does no harm.
// ---------------------------------------------------------------------------

import { VersionVector, type LoroDoc } from "loro-crdt";

/** Send one update. Resolve when the server accepted it; reject otherwise.
 *  A rejection with a numeric `status` is treated as the HTTP status. */
export type OutboxSend = (update: Uint8Array, options: { keepalive: boolean }) => Promise<void>;

/**
 * - `idle`: the server has every local edit.
 * - `sending`: a request is in flight.
 * - `retrying`: the last request failed and a retry is scheduled.
 * - `rejected`: the server refused the edits; nothing more is sent.
 */
export type OutboxState = "idle" | "sending" | "retrying" | "rejected";

export interface OutboxOptions {
  /** First retry delay. Doubles per failure up to `retryMaxMs`. */
  retryBaseMs?: number;
  retryMaxMs?: number;
  /** Timer functions, replaceable in tests. */
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
}

/** This peer's op counter in `doc` (exclusive end). */
export function ownCounter(doc: LoroDoc): number {
  return doc.oplogVersion().get(doc.peerIdStr) ?? 0;
}

/**
 * This peer's ops after counter `confirmed`, and no one else's: start from
 * everything the doc has, then roll this peer back to `confirmed`.
 */
export function exportOwnOps(doc: LoroDoc, confirmed: number): Uint8Array {
  const from = doc.oplogVersion().toJSON();
  if (confirmed > 0) from.set(doc.peerIdStr, confirmed);
  else from.delete(doc.peerIdStr);
  return doc.export({ mode: "update", from: VersionVector.parseJSON(from) });
}

/** True for an error the same request could get past by trying again. */
export function isRetryable(err: unknown): boolean {
  const status = (err as { status?: unknown } | null)?.status;
  if (typeof status !== "number") return true; // network error, abort, timeout
  return status >= 500 || status === 408 || status === 429;
}

export class CrdtOutbox {
  private readonly doc: LoroDoc;
  private readonly send: OutboxSend;
  private readonly retryBaseMs: number;
  private readonly retryMaxMs: number;
  private readonly setTimer: (fn: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;
  private readonly stopListening: () => void;
  private readonly listeners = new Set<() => void>();

  /** This peer's op counter the server has confirmed (exclusive end). */
  private confirmed = 0;
  private inFlight = false;
  private retryHandle: unknown = null;
  private failures = 0;
  private disposed = false;
  private _state: OutboxState = "idle";
  private _error: unknown = null;

  constructor(doc: LoroDoc, send: OutboxSend, options: OutboxOptions = {}) {
    this.doc = doc;
    this.send = send;
    this.retryBaseMs = options.retryBaseMs ?? 500;
    this.retryMaxMs = options.retryMaxMs ?? 10_000;
    this.setTimer = options.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
    this.clearTimer = options.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
    this.stopListening = doc.subscribeLocalUpdates(() => this.schedule());
    // Edits made before the outbox existed go out now.
    this.schedule();
  }

  get state(): OutboxState {
    return this._state;
  }

  /** The error that made the outbox `rejected` or `retrying`. */
  get error(): unknown {
    return this._error;
  }

  /** True while some local edit has not been confirmed by the server. */
  hasUnsent(): boolean {
    return this.ownCounter() > this.confirmed;
  }

  /** Call `listener` on every state change. Returns the unsubscribe. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Send unconfirmed edits now, unless a request is already in flight. */
  schedule(): void {
    if (this.disposed || this._state === "rejected") return;
    if (this.inFlight || this.retryHandle !== null) return; // picked up when that settles
    void this.flush(false);
  }

  /** Skip the backoff wait (the browser came back online). */
  retryNow(): void {
    if (this.retryHandle === null) return;
    this.clearTimer(this.retryHandle);
    this.retryHandle = null;
    this.schedule();
  }

  /**
   * Last chance before the page goes away: send what is unconfirmed with
   * `keepalive`, so the browser finishes the request after the page is gone.
   * Runs even while a request is in flight; the server merges duplicates.
   */
  flushOnHide(): void {
    if (this.disposed || this._state === "rejected") return;
    const update = this.unsent();
    if (!update) return;
    const target = this.ownCounter();
    this.send(update, { keepalive: true }).then(
      () => this.confirm(target),
      () => {
        /* the regular path retries if the page stays */
      },
    );
  }

  /** Stop listening and cancel a pending retry. Edits not sent are dropped. */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.stopListening();
    if (this.retryHandle !== null) {
      this.clearTimer(this.retryHandle);
      this.retryHandle = null;
    }
    this.listeners.clear();
  }

  private ownCounter(): number {
    return ownCounter(this.doc);
  }

  /** This peer's ops after `confirmed`, or null when there are none. */
  private unsent(): Uint8Array | null {
    if (!this.hasUnsent()) return null;
    return exportOwnOps(this.doc, this.confirmed);
  }

  private confirm(counter: number): void {
    if (counter > this.confirmed) this.confirmed = counter;
  }

  private async flush(keepalive: boolean): Promise<void> {
    const update = this.unsent();
    if (!update) {
      this.setState("idle", null);
      return;
    }
    const target = this.ownCounter();
    this.inFlight = true;
    this.setState("sending", null);
    try {
      await this.send(update, { keepalive });
      this.confirm(target);
      this.failures = 0;
    } catch (err) {
      this.inFlight = false;
      if (this.disposed) return;
      if (!isRetryable(err)) {
        this.setState("rejected", err);
        return;
      }
      this.failures += 1;
      const delay = Math.min(this.retryMaxMs, this.retryBaseMs * 2 ** (this.failures - 1));
      this.setState("retrying", err);
      this.retryHandle = this.setTimer(() => {
        this.retryHandle = null;
        this.schedule();
      }, delay);
      return;
    }
    this.inFlight = false;
    if (this.disposed) return;
    if (this.hasUnsent()) void this.flush(false);
    else this.setState("idle", null);
  }

  private setState(state: OutboxState, error: unknown): void {
    if (this._state === state && this._error === error) return;
    this._state = state;
    this._error = error;
    for (const listener of this.listeners) {
      try {
        listener();
      } catch (err) {
        console.warn("[loro] outbox listener threw:", err);
      }
    }
  }
}
