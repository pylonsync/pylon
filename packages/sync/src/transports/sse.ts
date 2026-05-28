// Server-Sent Events transport. One-way GET stream of change events
// from a dedicated `/events` endpoint on baseUrl-port+2.
//
// SSE has no uplink, so `send()` is a no-op — the engine continues to
// dispatch subscribe / presence / topic frames through this method but
// they don't reach the wire on this transport. Apps that need
// bidirectional should pick WebSocket; SSE is for read-only mirror
// clients (dashboards, monitors) that don't subscribe to anything
// individually.
//
// Reconnect: SSE has its own browser-level reconnect, but it can't see
// our auth token (no header on the GET retry), so we own the reconnect
// loop here too — close on error, jittered backoff, pull-on-reconnect.

import type { ChangeEvent } from "../types";
import { ReconnectBackoff } from "./reconnect";
import type { Transport, TransportHost } from "./types";

export class SseTransport implements Transport {
  private readonly host: TransportHost;
  private es: EventSource | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly backoff: ReconnectBackoff;

  constructor(host: TransportHost) {
    this.host = host;
    this.backoff = new ReconnectBackoff(host.reconnectDelayMs);
  }

  start(): void {
    if (!this.host.isRunning()) return;
    if (!this.host.isLeader()) return;
    if (this.es) return;

    const sseUrl = this.deriveSseUrl();
    try {
      const es = new EventSource(sseUrl);
      this.es = es;
      es.onopen = () => {
        this.host.setStatus("connected");
        this.backoff.reset();
        this.host.onConnected();
      };
      es.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data) as Record<string, unknown>;
          if (
            typeof msg.seq === "number" &&
            msg.seq > 0 &&
            typeof msg.entity === "string" &&
            typeof msg.kind === "string"
          ) {
            this.host.onChangeEvent(msg as unknown as ChangeEvent);
            return;
          }
          // Non-change envelopes are rare on SSE (no reactive queries,
          // no row-revoked typically) but route through anyway so
          // future protocol additions don't get silently dropped.
          this.host.onJsonMessage(msg);
        } catch {
          // Ignore malformed events.
        }
      };
      es.onerror = () => {
        try {
          es.close();
        } catch {
          /* may already be closed */
        }
        if (this.es === es) this.es = null;
        if (!this.host.isRunning()) return;
        this.host.setStatus("reconnecting");
        this.host.onDisconnected();
        this.scheduleReconnect();
      };
    } catch {
      // EventSource unavailable in this environment (Node, jsdom
      // without a polyfill). Surface as a no-op — the engine's
      // connect-status stays at whatever it was, and the caller can
      // notice and switch transports.
    }
  }

  stop(): void {
    if (this.es) {
      try {
        this.es.close();
      } catch {
        /* may already be closed */
      }
      this.es = null;
    }
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  /** SSE is one-way. The engine still calls send() for subscribe /
   *  presence / topic / ping, but those frames have nowhere to go.
   *  Silent no-op rather than throw: a no-op keeps app code identical
   *  across transports. */
  send(_msg: unknown): void {
    /* no uplink on SSE */
  }

  /** SSE is always "open enough" to receive once the EventSource is
   *  attached and not in CLOSED. We expose this for the engine's
   *  `connected` getter so UI status matches reality. */
  isOpen(): boolean {
    return (
      this.es !== null && this.es.readyState !== EventSource.CLOSED
    );
  }

  bumpReconnect(by = 1): void {
    this.backoff.bump(by);
  }

  // ---- internals --------------------------------------------------------

  private scheduleReconnect(): void {
    if (!this.host.isRunning()) return;
    this.backoff.bump();
    const delay = this.backoff.nextDelayMs();
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      void this.host.performReconnectPull().then(() => this.start());
    }, delay);
  }

  private deriveSseUrl(): string {
    const url = new URL(this.host.baseUrl);
    const port = parseInt(url.port || "4321", 10);
    return `http://${url.hostname}:${port + 2}/events`;
  }
}
