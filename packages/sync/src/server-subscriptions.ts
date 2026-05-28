// ServerSubscriptions — replay-on-reconnect registry shared by every
// server-side ephemeral subscription type.
//
// The server clears per-client subscription state on disconnect: CRDT
// row subscriptions, reactive query subscriptions, and any future kind
// (file streams, presence, etc.) all evaporate when the WS socket
// closes. Each kind used to track its own re-registration in a
// separate field on the engine, with its own loop in `ws.onopen`. This
// generalizes: every subscription kind records the exact WS message
// the server needs to re-create its server-side state; reconnect
// replays the bundle.
//
// Kind-specific concerns (CRDT refcount, reactive handler routing)
// stay in the engine. This module only owns the replay bookkeeping.

export class ServerSubscriptions {
  private specs = new Map<string, unknown>(); // key → subscribeMessage

  constructor(private readonly sendWs: (msg: unknown) => void) {}

  /** Register a subscription. Sends `subscribeMessage` over WS and
   *  remembers it so the next reconnect re-sends it.
   *
   *  Re-registering the same key with the SAME payload is a no-op
   *  (the prior subscribe is still live on the server). But a
   *  re-register with a DIFFERENT payload re-sends — that's the
   *  intended behavior of `useReactiveQuery(name, args)` when args
   *  change: same sub_id, new args, server must observe the change
   *  or the handler keeps running against stale arguments. */
  register(key: string, subscribeMessage: unknown): void {
    const prev = this.specs.get(key);
    const changed =
      prev === undefined || !sameJson(prev, subscribeMessage);
    this.specs.set(key, subscribeMessage);
    if (changed) this.sendWs(subscribeMessage);
  }

  /** Unregister. Sends `unsubscribeMessage` over WS and forgets the
   *  replay entry. No-op for unknown keys (matches React's
   *  StrictMode-friendly double-unmount semantics). */
  unregister(key: string, unsubscribeMessage: unknown): void {
    if (!this.specs.has(key)) return;
    this.specs.delete(key);
    this.sendWs(unsubscribeMessage);
  }

  /** Whether `key` is currently registered. */
  has(key: string): boolean {
    return this.specs.has(key);
  }

  /** Re-send every registered subscribe message. Called from
   *  `ws.onopen` after the socket reconnects — the server purges
   *  per-client subscription state on disconnect, so without this
   *  resync the subscriber's first event would never arrive. */
  replay(): void {
    for (const msg of this.specs.values()) {
      this.sendWs(msg);
    }
  }
}

/** Stable structural equality. Used to skip the WS round-trip when a
 *  re-register has the same payload — and to trigger one when it
 *  doesn't. Cheap enough on the small JSON shapes these messages use. */
function sameJson(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  return JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
}

function canonical(v: unknown): unknown {
  if (v === null || typeof v !== "object") return v;
  if (Array.isArray(v)) return v.map(canonical);
  const obj = v as Record<string, unknown>;
  const sorted: Record<string, unknown> = {};
  for (const k of Object.keys(obj).sort()) sorted[k] = canonical(obj[k]);
  return sorted;
}
