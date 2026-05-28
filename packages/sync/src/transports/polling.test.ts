// Unit tests for the PollingTransport in isolation — the engine
// integration is covered by `scenarios.test.ts` (which exercises
// poll-mode reconcile + race fixes). These tests just pin the
// transport's own contract:
//  - start() begins ticking on `pollIntervalMs`
//  - each tick calls host.performPollTick()
//  - stop() halts the loop
//  - send() / bumpReconnect() / onConnected are deterministic no-ops

import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { PollingTransport } from "./polling";
import type { TransportHost } from "./types";

function makeHost(overrides: Partial<TransportHost> = {}): TransportHost & {
  ticks: number;
} {
  const host = {
    baseUrl: "http://stub.invalid",
    pollIntervalMs: 20,
    getToken: () => undefined,
    isLeader: () => true,
    isRunning: () => true,
    onChangeEvent: () => {},
    onJsonMessage: () => {},
    onBinaryFrame: () => {},
    onConnected: () => {},
    onDisconnected: () => {},
    setStatus: () => {},
    performPollTick: async () => {
      host.ticks += 1;
    },
    performReconnectPull: async () => {},
    ticks: 0,
    ...overrides,
  };
  return host as TransportHost & { ticks: number };
}

describe("PollingTransport", () => {
  let t: PollingTransport | null = null;
  let host: ReturnType<typeof makeHost> | null = null;

  beforeEach(() => {
    host = makeHost();
    t = new PollingTransport(host);
  });

  afterEach(() => {
    t?.stop();
    t = null;
    host = null;
  });

  test("start kicks off a tick loop on pollIntervalMs cadence", async () => {
    t!.start();
    expect(host!.ticks).toBe(0); // no eager tick — first tick fires AFTER the interval
    await new Promise((r) => setTimeout(r, 70));
    // 70ms / 20ms = ~3 ticks. Allow some slop for timer jitter.
    expect(host!.ticks).toBeGreaterThanOrEqual(2);
    expect(host!.ticks).toBeLessThanOrEqual(5);
  });

  test("stop halts the loop", async () => {
    t!.start();
    await new Promise((r) => setTimeout(r, 40));
    const seen = host!.ticks;
    t!.stop();
    await new Promise((r) => setTimeout(r, 60));
    expect(host!.ticks).toBe(seen);
  });

  test("start is idempotent", () => {
    t!.start();
    t!.start();
    t!.start();
    // No assertion beyond not throwing — the second/third call should
    // see the existing timer and bail.
    expect(t!.isOpen()).toBe(true);
  });

  test("send is a no-op (polling has no uplink)", () => {
    t!.start();
    expect(() => t!.send({ type: "ping" })).not.toThrow();
  });

  test("bumpReconnect is a no-op (polling has no backoff)", () => {
    expect(() => t!.bumpReconnect(5)).not.toThrow();
  });

  test("does not start when host is not running", () => {
    host!.isRunning = () => false;
    t!.start();
    expect(t!.isOpen()).toBe(false);
  });

  test("does not start when host is not the multi-tab leader", () => {
    host!.isLeader = () => false;
    t!.start();
    expect(t!.isOpen()).toBe(false);
  });
});
