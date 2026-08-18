// Unit tests for the WebSocketTransport. Uses a fake WebSocket
// implementation so the test runs without a real server. Pins:
//  - start() opens a socket and wires onopen/onmessage/onclose/onerror
//  - onopen flips status to "connected", replay fires via host.onConnected
//  - binary frames route through host.onBinaryFrame
//  - ChangeEvent JSON routes through host.onChangeEvent
//  - Other JSON routes through host.onJsonMessage
//  - Disconnect triggers reconnect after a backoff window
//  - stop() suppresses the scheduled reconnect

import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  test,
} from "bun:test";

import type { ChangeEvent, SyncConnectionStatus } from "../types";
import type { TransportHost } from "./types";
import { WebSocketTransport } from "./websocket";

// ---------------------------------------------------------------------------
// Fake WebSocket. Captures every constructor call so tests can poke at the
// returned socket's onopen/onmessage/onclose/onerror handlers. The real
// browser API doesn't let us do that — but we control what `new WebSocket`
// returns in test land.
// ---------------------------------------------------------------------------

interface FakeWebSocketInstance {
  url: string;
  protocols?: string | string[];
  readyState: number;
  binaryType: string;
  sent: string[];
  onopen: ((ev: Event) => void) | null;
  onmessage: ((ev: MessageEvent) => void) | null;
  onclose: ((ev: CloseEvent) => void) | null;
  onerror: ((ev: Event) => void) | null;
  send(data: string): void;
  close(): void;
  /** Test helper: simulate the server pushing a frame. */
  simulateMessage(data: string | ArrayBuffer): void;
  /** Test helper: simulate the open handshake completing. */
  simulateOpen(): void;
  /** Test helper: simulate an unexpected close. */
  simulateClose(): void;
}

const fakeSockets: FakeWebSocketInstance[] = [];

class FakeWebSocket implements FakeWebSocketInstance {
  url: string;
  protocols?: string | string[];
  readyState = 0; // CONNECTING
  binaryType = "blob";
  sent: string[] = [];
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;

  constructor(url: string, protocols?: string | string[]) {
    this.url = url;
    this.protocols = protocols;
    fakeSockets.push(this);
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = 3; // CLOSED
    // Bun's tests run synchronously enough that we don't fire onclose
    // here — the production code's stop() nulls onclose before this
    // is called, and the test's simulateClose covers the other path.
  }

  simulateOpen(): void {
    this.readyState = 1; // OPEN
    this.onopen?.(new Event("open"));
  }

  simulateMessage(data: string | ArrayBuffer): void {
    this.onmessage?.({ data } as MessageEvent);
  }

  simulateClose(): void {
    this.readyState = 3; // CLOSED
    this.onclose?.(new CloseEvent("close"));
  }
}

// Patch globals once for this file. We intentionally don't restore —
// the test runner gives each file its own module evaluation, and other
// test files that need a real WebSocket already supply their own stub
// (e.g., the scenario harness installs its own). Restoring after each
// test caused us to clobber the FakeWebSocket mid-suite and break
// subsequent tests in this file.
beforeAll(() => {
  Object.defineProperty(globalThis, "WebSocket", {
    value: FakeWebSocket,
    writable: true,
    configurable: true,
  });
  // The transport checks `this.ws.readyState === WebSocket.OPEN`, so
  // the static .OPEN constant has to exist on whatever class we
  // install. Mirrors the browser's WebSocket.OPEN === 1.
  Object.defineProperty(FakeWebSocket, "OPEN", {
    value: 1,
    writable: false,
    configurable: true,
  });
});

// ---------------------------------------------------------------------------

interface TestState {
  statuses: SyncConnectionStatus[];
  changes: ChangeEvent[];
  jsonMessages: Record<string, unknown>[];
  binaries: Uint8Array[];
  connectedCount: number;
  reconnectPulls: number;
}

/** Build a TransportHost wired to a TestState recorder. Recorder is
 *  returned via `state` so tests can read counters after the transport
 *  has called back into the host. Spreading state into host would
 *  copy the counter values once at construction — the recorder needs
 *  shared mutable state. */
function makeHost(
  overrides: Partial<TransportHost> = {},
): { host: TransportHost; state: TestState } {
  const state: TestState = {
    statuses: [],
    changes: [],
    jsonMessages: [],
    binaries: [],
    connectedCount: 0,
    reconnectPulls: 0,
  };
  const host: TransportHost = {
    baseUrl: "http://stub.invalid",
    reconnectDelayMs: 10,
    getToken: () => undefined,
    isLeader: () => true,
    isRunning: () => true,
    onChangeEvent: (ev) => state.changes.push(ev),
    onJsonMessage: (msg) => state.jsonMessages.push(msg),
    onBinaryFrame: (bytes) => state.binaries.push(bytes),
    onConnected: () => {
      state.connectedCount += 1;
    },
    onDisconnected: () => {},
    setStatus: (s) => state.statuses.push(s),
    performPollTick: async () => {},
    performReconnectPull: async () => {
      state.reconnectPulls += 1;
    },
    ...overrides,
  };
  return { host, state };
}

describe("WebSocketTransport", () => {
  let t: WebSocketTransport | null = null;

  beforeEach(() => {
    fakeSockets.length = 0;
    t = null;
  });

  afterEach(() => {
    t?.stop();
    t = null;
  });

  test("start opens a socket with the derived URL", () => {
    const { host, state } = makeHost();
    t = new WebSocketTransport(host);
    t.start();
    expect(fakeSockets.length).toBe(1);
    // baseUrl is http → ws scheme; multiplexed on /api/sync/ws.
    expect(fakeSockets[0].url).toBe("ws://stub.invalid/api/sync/ws");
  });

  test("start passes the bearer-subprotocol when host has a token", () => {
    const { host } = makeHost({ getToken: () => "tok_abc" });
    t = new WebSocketTransport(host);
    t.start();
    expect(fakeSockets[0].protocols).toBe("bearer.tok_abc");
  });

  test("relay mode dials the minted URL with the blob as a subprotocol", async () => {
    let mints = 0;
    const { host } = makeHost({
      getToken: () => "tok_abc", // machine token must NOT reach the wire
      getRelayTarget: async () => {
        mints += 1;
        return {
          url: `wss://relay.invalid/sync/ws?app=demo&since=42`,
          blob: `blob${mints}`,
        };
      },
    });
    t = new WebSocketTransport(host);
    t.start();
    // Resolution is async; nothing dialed synchronously, and a second
    // start() during resolution must not stack a second mint.
    expect(fakeSockets.length).toBe(0);
    t.start();
    await Promise.resolve();
    await Promise.resolve();
    expect(mints).toBe(1);
    expect(fakeSockets.length).toBe(1);
    // The blob is NOT in the URL (stays out of proxy logs) …
    expect(fakeSockets[0].url).toBe("wss://relay.invalid/sync/ws?app=demo&since=42");
    // … it rides the bearer subprotocol, and the machine token doesn't.
    expect(fakeSockets[0].protocols).toBe("bearer.blob1");
  });

  test("relay mint failure backs off instead of dialing", async () => {
    const { host } = makeHost({ getRelayTarget: async () => null });
    t = new WebSocketTransport(host);
    t.start();
    await Promise.resolve();
    await Promise.resolve();
    expect(fakeSockets.length).toBe(0);
    // Backoff timer armed: a later fire re-attempts via reconnect pull.
    await new Promise((r) => setTimeout(r, 30));
    // Each retry re-mints (and fails) — no socket ever opens.
    expect(fakeSockets.length).toBe(0);
  });

  test("onopen flips status to connected and fires host.onConnected", () => {
    const { host, state } = makeHost();
    t = new WebSocketTransport(host);
    t.start();
    fakeSockets[0].simulateOpen();
    expect(state.statuses).toContain("connected");
    expect(state.connectedCount).toBe(1);
  });

  test("ChangeEvent frames route through onChangeEvent", () => {
    const { host, state } = makeHost();
    t = new WebSocketTransport(host);
    t.start();
    fakeSockets[0].simulateOpen();
    const ev = {
      seq: 5,
      entity: "Todo",
      row_id: "r1",
      kind: "insert",
      data: { id: "r1" },
      timestamp: "",
    };
    fakeSockets[0].simulateMessage(JSON.stringify(ev));
    expect(state.changes.length).toBe(1);
    expect(state.changes[0].seq).toBe(5);
  });

  test("non-change JSON routes through onJsonMessage", () => {
    const { host, state } = makeHost();
    t = new WebSocketTransport(host);
    t.start();
    fakeSockets[0].simulateOpen();
    fakeSockets[0].simulateMessage(
      JSON.stringify({ type: "session-changed" }),
    );
    expect(state.jsonMessages.length).toBe(1);
    expect(state.jsonMessages[0].type).toBe("session-changed");
    expect(state.changes.length).toBe(0);
  });

  test("binary frames route through onBinaryFrame", () => {
    const { host, state } = makeHost();
    t = new WebSocketTransport(host);
    t.start();
    fakeSockets[0].simulateOpen();
    const bytes = new Uint8Array([1, 2, 3, 4]).buffer;
    fakeSockets[0].simulateMessage(bytes);
    expect(state.binaries.length).toBe(1);
    expect(state.binaries[0].length).toBe(4);
  });

  test("send writes a JSON string to the socket when open", () => {
    const { host, state } = makeHost();
    t = new WebSocketTransport(host);
    t.start();
    fakeSockets[0].simulateOpen();
    t.send({ type: "presence", x: 1 });
    expect(fakeSockets[0].sent).toContain('{"type":"presence","x":1}');
  });

  test("send is a no-op when the socket isn't open", () => {
    const { host, state } = makeHost();
    t = new WebSocketTransport(host);
    t.start();
    // Don't simulateOpen — readyState stays at CONNECTING.
    t.send({ type: "presence" });
    expect(fakeSockets[0].sent.length).toBe(0);
  });

  test("unexpected close triggers reconnect via performReconnectPull", async () => {
    const { host, state } = makeHost({ reconnectDelayMs: 5 });
    t = new WebSocketTransport(host);
    t.start();
    fakeSockets[0].simulateOpen();
    fakeSockets[0].simulateClose();
    // Wait long enough for the backoff timer + reconnect to fire.
    await new Promise((r) => setTimeout(r, 80));
    expect(state.reconnectPulls).toBeGreaterThanOrEqual(1);
    // A second socket should have been opened.
    expect(fakeSockets.length).toBeGreaterThanOrEqual(2);
  });

  test("stop suppresses the scheduled reconnect", async () => {
    const { host, state } = makeHost({ reconnectDelayMs: 5 });
    t = new WebSocketTransport(host);
    t.start();
    fakeSockets[0].simulateOpen();
    fakeSockets[0].simulateClose();
    t.stop();
    await new Promise((r) => setTimeout(r, 80));
    // No reconnect pull should have fired — stop cancelled the timer.
    expect(state.reconnectPulls).toBe(0);
  });

  test("does not open a socket when host is not leader", () => {
    const { host } = makeHost({ isLeader: () => false });
    t = new WebSocketTransport(host);
    t.start();
    expect(fakeSockets.length).toBe(0);
  });

  test("isOpen reflects the underlying readyState", () => {
    const { host, state } = makeHost();
    t = new WebSocketTransport(host);
    expect(t.isOpen()).toBe(false);
    t.start();
    expect(t.isOpen()).toBe(false); // CONNECTING
    fakeSockets[0].simulateOpen();
    expect(t.isOpen()).toBe(true); // OPEN
    fakeSockets[0].simulateClose();
    expect(t.isOpen()).toBe(false); // CLOSED
  });
});

