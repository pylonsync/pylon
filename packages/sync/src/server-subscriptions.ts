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
   *  remembers it so the next reconnect re-sends it. Idempotent —
   *  registering the same key twice replaces the message and does
   *  NOT re-send (the prior subscribe is still live on the server). */
  register(key: string, subscribeMessage: unknown): void {
    const wasNew = !this.specs.has(key);
    this.specs.set(key, subscribeMessage);
    if (wasNew) this.sendWs(subscribeMessage);
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
