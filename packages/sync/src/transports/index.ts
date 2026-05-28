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
 *  underlying mechanism. */
export function createTransport(
  kind: TransportKind,
  host: TransportHost,
): Transport {
  switch (kind) {
    case "websocket":
      return new WebSocketTransport(host);
    case "sse":
      return new SseTransport(host);
    case "poll":
      return new PollingTransport(host);
  }
}
