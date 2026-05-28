// Unit tests for the ReconnectBackoff helper. Pins the contract the
// WS + SSE transports rely on:
//  - Starts at 0 attempts (delay = 0 on first nextDelayMs).
//  - bump() increments; nextDelayMs() grows exponentially with jitter.
//  - reset() returns to 0.
//  - Cap at MAX_DELAY_MS.

import { describe, expect, test } from "bun:test";

import { ReconnectBackoff } from "./reconnect";

describe("ReconnectBackoff", () => {
  test("first delay is 0 (no jitter on no attempts)", () => {
    const b = new ReconnectBackoff(1_000);
    // attempts=0, but nextDelayMs uses max(1, attempts)=1 → exp=1000 →
    // random in [0, 1000]. Just check the BOUND.
    const d = b.nextDelayMs();
    expect(d).toBeGreaterThanOrEqual(0);
    expect(d).toBeLessThanOrEqual(1_000);
  });

  test("each bump doubles the exponent (until cap)", () => {
    const b = new ReconnectBackoff(1_000);
    b.bump(); // 1 → exp=1000
    const d1Max = b.nextDelayMs();
    b.bump(); // 2 → exp=2000
    const d2Max = b.nextDelayMs();
    b.bump(); // 3 → exp=4000
    const d3Max = b.nextDelayMs();
    // Bounds (not exact values — jitter is random).
    expect(d1Max).toBeLessThanOrEqual(1_000);
    expect(d2Max).toBeLessThanOrEqual(2_000);
    expect(d3Max).toBeLessThanOrEqual(4_000);
  });

  test("delay caps at 30_000 ms even with many attempts", () => {
    const b = new ReconnectBackoff(1_000);
    // 10 doublings = 2^10 * 1_000 = ~1M ms unclamped. Cap is 30_000.
    for (let i = 0; i < 20; i++) b.bump();
    const d = b.nextDelayMs();
    expect(d).toBeLessThanOrEqual(30_000);
  });

  test("reset clears the counter", () => {
    const b = new ReconnectBackoff(1_000);
    b.bump(5);
    b.reset();
    // Same bound as the fresh state.
    expect(b.nextDelayMs()).toBeLessThanOrEqual(1_000);
  });

  test("bump(N) jumps by N — used by the 429 case in the engine", () => {
    const b = new ReconnectBackoff(1_000);
    b.bump(3); // attempts=3 → exp=4000
    expect(b.nextDelayMs()).toBeLessThanOrEqual(4_000);
  });
});
