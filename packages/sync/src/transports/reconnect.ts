// Shared exponential-backoff-with-full-jitter helper used by WS and
// SSE. Polling has its own steady-cadence loop and doesn't backoff.
//
// Thundering-herd fix: when the server restarts, every connected
// client fires onclose at nearly the same instant. Without jitter
// they all reconnect at `baseDelay` and hammer the newly-booted
// server; after a few cycles the reconnect waves align and the
// server never recovers. Full-jitter (`delay = random(0, exp)`)
// spreads clients evenly across the backoff window so the second-
// wave load is flat, not spiky. Algorithm from AWS Architecture
// Blog "Exponential Backoff and Jitter" — the "Full Jitter" variant.

const DEFAULT_BASE_DELAY_MS = 1_000;
const MAX_DELAY_MS = 30_000;

/** Stateful backoff counter. Reset to 0 attempts after a successful
 *  stable connection window. Increment on every failed connect /
 *  unexpected close. */
export class ReconnectBackoff {
  private attempts = 0;
  private baseDelayMs: number;

  constructor(baseDelayMs?: number) {
    this.baseDelayMs = baseDelayMs ?? DEFAULT_BASE_DELAY_MS;
  }

  /** Record a failed connect / unexpected close. Returns the new attempt
   *  count so callers can use it for bookkeeping (e.g., the 429 case
   *  bumps by 3 to push the next delay further out). */
  bump(by = 1): number {
    this.attempts += by;
    return this.attempts;
  }

  /** Reset the counter — call this after the connection has been
   *  stable long enough that subsequent reconnects shouldn't carry
   *  the prior backoff over. */
  reset(): void {
    this.attempts = 0;
  }

  /** Compute the next delay in ms. Always returns at least 0; never
   *  negative. Full-jitter over [0, exp] where exp grows
   *  exponentially with the attempt count and caps at MAX_DELAY_MS. */
  nextDelayMs(): number {
    const attempt = Math.max(1, this.attempts);
    const exp = Math.min(MAX_DELAY_MS, this.baseDelayMs * Math.pow(2, attempt - 1));
    return Math.floor(Math.random() * exp);
  }
}
