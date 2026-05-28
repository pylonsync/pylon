// Public surface for the transport layer. The engine imports types
// from here and constructs a transport via `createTransport(kind, host)`.

import { PollingTransport } from "./polling";
import { SseTransport } from "./sse";
import type { Transport, TransportHost } from "./types";
import { WebSocketTransport } from "./websocket";

export type TransportKind = "websocket" | "sse" | "poll";
export type { Transport, TransportHost } from "./types";
export { WebSocketTransport } from "./websocket";
export { SseTransport } from "./sse";
export { PollingTransport } from "./polling";

/** Build the right transport for a given kind. The host supplies all
 *  config + the inbound dispatch callbacks; the transport hides the
 *  underlying mechanism.
 *
 *  Fallback rule: `transport: "sse"` in an environment without a
 *  native EventSource (Node, jsdom without a polyfill, old browsers)
 *  silently downgrades to polling. The pre-refactor `connectSse()`
 *  did this inside its constructor catch — we lift the decision to
 *  the factory so the engine never sees an SseTransport that can
 *  never connect. Clients that NEED SSE specifically can detect this
 *  by feature-checking `typeof EventSource` themselves before calling
 *  `init()`. */
export function createTransport(
  kind: TransportKind,
  host: TransportHost,
): Transport {
  switch (kind) {
    case "websocket":
      return new WebSocketTransport(host);
    case "sse":
      if (typeof EventSource === "undefined") {
        return new PollingTransport(host);
      }
      return new SseTransport(host);
    case "poll":
      return new PollingTransport(host);
  }
}
