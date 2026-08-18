// WebSocket transport. Owns the active socket, the keepalive ping
// timer, the stable-connection timer (5s window before backoff resets),
// and reconnect scheduling.
//
// Frame routing: this transport only does the bare minimum — recognize
// binary vs JSON and call back into the host (engine) for actual
// dispatch. The host has the typed envelope table (change events,
// reactive results, row revocations, session-changed, pong, presence).
//
// Token handling: browser WebSocket has no header API, so we pass the
// bearer token as a `bearer.<percent-encoded>` subprotocol per
// RFC 6455 §1.9. Native clients can still use Authorization: Bearer.

import type { ChangeEvent } from "../types";
import { ReconnectBackoff } from "./reconnect";
import type { Transport, TransportHost } from "./types";

const STABLE_CONNECTION_MS = 5_000;
const DEFAULT_PING_INTERVAL_MS = 25_000;

export class WebSocketTransport implements Transport {
  private readonly host: TransportHost;
  private ws: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private stableTimer: ReturnType<typeof setTimeout> | null = null;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private resolvingRelay = false;
  private readonly backoff: ReconnectBackoff;

  constructor(host: TransportHost) {
    this.host = host;
    this.backoff = new ReconnectBackoff(host.reconnectDelayMs);
  }

  start(): void {
    if (!this.host.isRunning()) return;
    // Followers never open a WS — the leader's fanout is mirrored over
    // the BroadcastChannel.
    if (!this.host.isLeader()) return;
    // Already connected / connecting? Don't stack a second socket.
    if (this.ws || this.resolvingRelay) return;

    // Relay mode: the connect target is minted per-attempt (signed
    // blob + since cursor). The blob travels in the `bearer.<blob>`
    // subprotocol via connectTo, NOT the URL — keeping the live
    // credential out of proxy/CDN access logs.
    if (this.host.getRelayTarget) {
      this.resolvingRelay = true;
      this.host
        .getRelayTarget()
        .then((target) => {
          this.resolvingRelay = false;
          if (!this.host.isRunning() || this.ws) return;
          if (!target) {
            this.scheduleReconnect();
            return;
          }
          this.connectTo(target.url, target.blob);
        })
        .catch(() => {
          // getRelayTarget shouldn't reject (the engine's impl catches
          // to null), but a custom host might — never wedge on a stuck
          // resolvingRelay flag.
          this.resolvingRelay = false;
          this.scheduleReconnect();
        });
      return;
    }

    this.connectTo(this.host.wsUrl ?? this.deriveWsUrl(), this.host.getToken());
  }

  private connectTo(wsUrl: string, token: string | undefined): void {
    try {
      if (token) {
        const proto = `bearer.${encodeURIComponent(token)}`;
        this.ws = new WebSocket(wsUrl, proto);
      } else {
        this.ws = new WebSocket(wsUrl);
      }
    } catch {
      this.scheduleReconnect();
      return;
    }

    this.ws.binaryType = "arraybuffer";

    this.ws.onopen = () => {
      // Flip to "connected" immediately so UI consumers can clear the
      // "reconnecting" indicator the moment data starts flowing again.
      // The stable-window timer below decides when to RESET the
      // backoff counter — a socket that opens then closes inside a
      // few seconds (auth failure, server 1008) would otherwise let
      // the reconnect loop fire at ~2/sec forever.
      this.host.setStatus("connected");
      if (this.stableTimer) clearTimeout(this.stableTimer);
      this.stableTimer = setTimeout(() => {
        this.backoff.reset();
        this.stableTimer = null;
      }, STABLE_CONNECTION_MS);

      // Client-side keepalive ping. Default 25s — pure liveness, since
      // the dedicated :port+1 server uses a dual-thread design that
      // wakes the writer instantly on every broadcast. The HTTP-
      // multiplexed `/api/sync/ws` fallback path IS bounded by this
      // interval, so apps that can't route to the :port+1 listener
      // can pass `init({ pingIntervalMs: 200 })` to trade traffic for
      // latency.
      const pingIntervalMs = this.host.pingIntervalMs ?? DEFAULT_PING_INTERVAL_MS;
      if (this.pingTimer) clearInterval(this.pingTimer);
      this.pingTimer = setInterval(() => {
        if (this.ws?.readyState !== WebSocket.OPEN) return;
        try {
          this.ws.send('{"type":"ping"}');
        } catch {
          // ignore — onclose will trigger reconnect
        }
      }, pingIntervalMs);

      this.host.onConnected();
    };

    this.ws.onmessage = (event) => {
      // Binary frame: hand off to the host. Pylon ships every CRDT-mode
      // write as a binary [type|entity|row_id|payload] frame; @pylonsync/
      // loro is the intended decoder. The transport stays binary-agnostic
      // so future binary use cases (file streaming, video chunks…) can
      // register on the host side without churning this layer.
      if (event.data instanceof ArrayBuffer) {
        this.host.onBinaryFrame(new Uint8Array(event.data));
        return;
      }
      try {
        const msg = JSON.parse(event.data as string) as Record<string, unknown>;
        // Change event sniff: `seq + entity + kind` is the canonical
        // wire shape. Route through onChangeEvent so the host applies
        // through its shared apply queue (preserves seq order with
        // pull() output).
        if (
          typeof msg.seq === "number" &&
          msg.seq > 0 &&
          typeof msg.entity === "string" &&
          typeof msg.kind === "string"
        ) {
          this.host.onChangeEvent(msg as unknown as ChangeEvent);
          return;
        }
        this.host.onJsonMessage(msg);
      } catch {
        // Ignore malformed messages.
      }
    };

    this.ws.onclose = () => {
      this.ws = null;
      // Socket closed before the stable-window timer fired — treat
      // this as an unstable connection and DON'T reset the backoff.
      // The growing delay protects the server from a tight loop.
      if (this.stableTimer) {
        clearTimeout(this.stableTimer);
        this.stableTimer = null;
      }
      if (this.pingTimer) {
        clearInterval(this.pingTimer);
        this.pingTimer = null;
      }
      if (this.host.isRunning()) {
        this.host.setStatus("reconnecting");
      }
      this.host.onDisconnected();
      this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      // onclose will fire after this — no work to do here.
    };
  }

  stop(): void {
    if (this.ws) {
      // Null-out onclose so the close we're triggering doesn't kick
      // off another reconnect cycle.
      this.ws.onclose = null;
      try {
        this.ws.close();
      } catch {
        /* socket may already be torn down */
      }
      this.ws = null;
    }
    // A relay-target resolution in flight must not re-open after stop.
    this.resolvingRelay = false;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.stableTimer) {
      clearTimeout(this.stableTimer);
      this.stableTimer = null;
    }
    if (this.pingTimer) {
      clearInterval(this.pingTimer);
      this.pingTimer = null;
    }
  }

  send(msg: unknown): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }

  isOpen(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  /** Bump the reconnect counter explicitly. Used for the 429 RESYNC
   *  case where the engine wants the next delay to be 3 steps further
   *  out — protects the server from a 429-loop where a 429 on pull
   *  triggers onclose → reconnect → pull → 429 in a tight cycle. */
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
      // Pull any missed changes before the next connect attempt, then
      // reconnect. The pull is engine-side so retries / 410 handling
      // / session resets stay centralized.
      void this.host.performReconnectPull().then(() => this.start());
    }, delay);
  }

  private deriveWsUrl(): string {
    const url = new URL(this.host.baseUrl);
    const scheme = url.protocol === "https:" ? "wss" : "ws";
    // Always multiplex WS on the same origin via `/api/sync/ws`. The
    // Pylon runtime accepts the Upgrade on its main HTTP port, so any
    // reverse proxy that already forwards `/api/*` carries the
    // WebSocket through too (Vite's `ws: true` proxy, Next.js
    // rewrites, CDNs with WS support). The legacy port+1 fallback is
    // still available on the runtime; we just don't derive it client-
    // side anymore because any setup where the page origin wasn't
    // equal to the API origin would compute ws://localhost:3001 — which
    // doesn't exist and bypasses the dev-server proxy.
    return `${scheme}://${url.host}/api/sync/ws`;
  }
}
