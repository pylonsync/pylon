import { describe, expect, test } from "bun:test";

import { ShardClock } from "./clock";

/** A deterministic jitter source in [0, 1). */
function rng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 2 ** 32;
  };
}

/**
 * Frames for ticks `from..to` at 20 Hz, built at `tick * 50` ms, arriving
 * after `latency` plus up to `jitter` ms.
 */
function feed(
  clock: ShardClock,
  from: number,
  to: number,
  latency: number,
  jitter: number,
  seed = 1,
) {
  const r = rng(seed);
  for (let tick = from; tick <= to; tick++) clock.observe(tick, tick * 50 + latency + r() * jitter);
}

describe("ShardClock", () => {
  test("is not ready before a frame", () => {
    const clock = new ShardClock();
    expect(clock.ready).toBe(false);
    expect(clock.serverTick(1000)).toBe(-1);
  });

  test("anchors on the least-delayed frame, so jitter does not move it", () => {
    const clock = new ShardClock({ tickRate: 20 });
    feed(clock, 1, 200, 40, 60);
    // At time t the newest tick that could have arrived was built at
    // t - 40 ms (the smallest delay seen is close to 40).
    const est = clock.serverTick(200 * 50 + 40 + 25);
    expect(est).toBeGreaterThan(200.3);
    expect(est).toBeLessThan(200.6);
  });

  test("measures the tick length when the rate is unknown", () => {
    const clock = new ShardClock();
    const r = rng(7);
    for (let tick = 1; tick <= 128; tick++) clock.observe(tick, tick * 33.3 + 20 + r() * 15);
    expect(clock.tickMs).toBeGreaterThan(32.8);
    expect(clock.tickMs).toBeLessThan(33.8);
  });

  test("never goes backwards when a correction arrives", () => {
    const clock = new ShardClock({ tickRate: 20 });
    feed(clock, 1, 64, 100, 0);
    let t = 64 * 50 + 100;
    let prev = clock.serverTick(t);
    // Latency drops by 60 ms: the estimate must move ahead, gradually.
    for (let tick = 65; tick <= 200; tick++) {
      clock.observe(tick, tick * 50 + 40);
      for (let step = 0; step < 3; step++) {
        t += 50 / 3;
        const now = clock.serverTick(t);
        expect(now).toBeGreaterThan(prev);
        prev = now;
      }
    }
    // It converged on the new, lower latency.
    expect(clock.serverTick(200 * 50 + 40)).toBeCloseTo(200, 1);
  });

  test("follows a shard whose ticks resume late after a stall", () => {
    const clock = new ShardClock({ tickRate: 20 });
    feed(clock, 1, 40, 50, 0);
    // The tick loop stalled for 2 s, then kept its tick numbers going.
    for (let tick = 41; tick <= 43; tick++) clock.observe(tick, tick * 50 + 50 + 2000);
    expect(clock.serverTick(43 * 50 + 50 + 2000)).toBeCloseTo(43, 1);
  });

  test("one late frame is jitter", () => {
    const clock = new ShardClock({ tickRate: 20 });
    feed(clock, 1, 40, 50, 0);
    clock.observe(41, 41 * 50 + 50 + 2000);
    clock.observe(42, 42 * 50 + 50);
    expect(clock.serverTick(42 * 50 + 50)).toBeCloseTo(42, 1);
  });

  test("a burst after a network stall catches up to the newest tick", () => {
    const clock = new ShardClock({ tickRate: 20 });
    feed(clock, 1, 40, 50, 0);
    // Ticks 41..80 were built on time but arrive together, 2 s late.
    const at = 40 * 50 + 50 + 2000;
    for (let tick = 41; tick <= 80; tick++) clock.observe(tick, at);
    // Within a tick at once; the rest is spread over the next second.
    expect(clock.serverTick(at)).toBeGreaterThan(78.9);
    expect(clock.serverTick(at + 1500)).toBeCloseTo(110, 1);
  });

  test("ticks that go back (a restarted shard) reset it", () => {
    const clock = new ShardClock({ tickRate: 20 });
    feed(clock, 1000, 1010, 30, 0);
    clock.observe(3, 999_999);
    expect(clock.latestTick).toBe(3);
    expect(clock.serverTick(999_999)).toBeCloseTo(3);
  });
});
