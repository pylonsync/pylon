// Tests for the createTransport factory + its sse→polling fallback.
//
// Regression for codex round-7 P2: pre-refactor, `transport: "sse"` in
// an environment without a native `EventSource` (Node / jsdom /
// unsupported browser) silently fell back to polling via the catch
// block inside `connectSse()`. The extraction dropped that path; the
// factory restores it by feature-checking up front so the engine
// never holds an SseTransport that can never connect.

import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import {
  createTransport,
  PollingTransport,
  SseTransport,
  WebSocketTransport,
} from "./index";
import type { TransportHost } from "./types";

function noopHost(): TransportHost {
  return {
    baseUrl: "http://stub.invalid",
    getToken: () => undefined,
    isLeader: () => true,
    isRunning: () => true,
    onChangeEvent: () => {},
    onJsonMessage: () => {},
    onBinaryFrame: () => {},
    onConnected: () => {},
    onDisconnected: () => {},
    setStatus: () => {},
    performPollTick: async () => {},
    performReconnectPull: async () => {},
  };
}

describe("createTransport factory", () => {
  test("websocket kind builds a WebSocketTransport", () => {
    const t = createTransport("websocket", noopHost());
    expect(t).toBeInstanceOf(WebSocketTransport);
  });

  test("poll kind builds a PollingTransport", () => {
    const t = createTransport("poll", noopHost());
    expect(t).toBeInstanceOf(PollingTransport);
  });
});

describe("createTransport sse → polling fallback", () => {
  let originalEventSource: unknown;

  beforeEach(() => {
    originalEventSource = (globalThis as { EventSource?: unknown }).EventSource;
  });

  afterEach(() => {
    if (originalEventSource === undefined) {
      delete (globalThis as { EventSource?: unknown }).EventSource;
    } else {
      (globalThis as { EventSource?: unknown }).EventSource =
        originalEventSource;
    }
  });

  test("sse kind WITH EventSource defined → SseTransport", () => {
    // Provide a minimal stub so `typeof EventSource !== "undefined"`.
    (globalThis as { EventSource: unknown }).EventSource = class {
      // constructor is enough; we don't actually start the transport here
      // (start would try to open a real connection). The factory check
      // is purely a `typeof` lookup.
    };
    const t = createTransport("sse", noopHost());
    expect(t).toBeInstanceOf(SseTransport);
  });

  test("sse kind WITHOUT EventSource → PollingTransport (fallback)", () => {
    // Bun's test runtime has no global EventSource by default. Make sure
    // it's actually undefined for this test, then assert the fallback
    // kicks in.
    delete (globalThis as { EventSource?: unknown }).EventSource;
    const t = createTransport("sse", noopHost());
    expect(t).toBeInstanceOf(PollingTransport);
    // The pre-refactor behavior this pins: a Node consumer that calls
    // `init({ transport: "sse" })` keeps syncing via polling instead of
    // sitting in `connecting` forever with no socket and no timer.
  });
});
