// Polling transport. No persistent connection — every `pollIntervalMs`
// the transport asks the host to do one push→pull cycle. Used by
// environments where WS / SSE are unavailable (corporate proxies that
// strip the Upgrade header, ancient browsers, certain serverless edge
// runtimes).
//
// `send()` is a no-op — no socket exists. Subscribe / presence / topic
// frames silently disappear. Apps that need bidirectional should not
// use polling.

import type { Transport, TransportHost } from "./types";

const DEFAULT_POLL_INTERVAL_MS = 1_000;

export class PollingTransport implements Transport {
  private readonly host: TransportHost;
  private timer: ReturnType<typeof setInterval> | null = null;

  constructor(host: TransportHost) {
    this.host = host;
  }

  start(): void {
    if (!this.host.isRunning()) return;
    if (!this.host.isLeader()) return;
    if (this.timer) return;
    const interval = this.host.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
    this.timer = setInterval(() => {
      void this.host.performPollTick();
    }, interval);
    // Polling intentionally does NOT fire onConnected on start — there
    // is no real "open" event with this transport, just a heartbeat
    // tick every `pollIntervalMs`. Engine status stays at whatever
    // `start()` set it to (typically "connecting"), and each
    // successful tick implicitly proves liveness without us claiming
    // a connection that doesn't exist. Apps that need an explicit
    // "online" indicator under polling can derive it from the
    // store's `notify` cadence.
  }

  stop(): void {
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }

  send(_msg: unknown): void {
    /* no uplink on polling */
  }

  isOpen(): boolean {
    return this.timer !== null;
  }

  /** No-op — polling doesn't have a backoff counter; it just retries
   *  every interval. A 429 on the in-flight push/pull will surface as
   *  a thrown error on the next tick but the loop continues at the
   *  same cadence. */
  bumpReconnect(_by = 1): void {
    /* polling has no backoff */
  }
}
