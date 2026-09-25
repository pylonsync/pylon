import { describe, expect, test } from "bun:test";

import { replicationPayload, type TestFrame } from "../test/frames";
import { EntityInterpolator, type InterpolationOptions } from "./interpolation";
import { EntityTable } from "./replication";

/** Feeds frames to a table and an interpolator, tick by tick. */
function harness(options?: InterpolationOptions) {
  const table = new EntityTable();
  const interp = new EntityInterpolator(options);
  return {
    interp,
    frame(tick: number, f: TestFrame) {
      interp.record(table, table.apply(replicationPayload(f)), tick);
    },
    at(renderTick: number) {
      interp.update(renderTick);
      return interp.entities;
    },
  };
}

describe("EntityInterpolator", () => {
  test("draws an entity between the samples around the render tick", () => {
    const h = harness();
    h.frame(1, { full: true, spawn: [{ id: 1, pos: [0, 0, 0] }] });
    h.frame(2, { update: [{ id: 1, from: [0, 0, 0], pos: [10, 0, 0] }] });
    h.frame(3, { update: [{ id: 1, from: [10, 0, 0], pos: [20, -4, 0] }] });
    expect(h.at(1.5).get(1)?.x).toBeCloseTo(5);
    const e = h.at(2.25).get(1)!;
    expect(e.x).toBeCloseTo(12.5);
    expect(e.y).toBeCloseTo(-1);
    expect([e.from.tick, e.to.tick, e.t]).toEqual([2, 3, 0.25]);
    // Past the newest sample it stays there.
    expect(h.at(7).get(1)?.x).toBeCloseTo(20);
  });

  test("an entity appears at its spawn tick and leaves at its despawn tick", () => {
    const h = harness();
    h.frame(1, { full: true });
    h.frame(5, { spawn: [{ id: 3, pos: [1, 1, 1] }] });
    h.frame(8, { despawn: [3] });
    expect(h.at(4.9).has(3)).toBe(false);
    h.at(5);
    expect(h.interp.entered).toEqual([3]);
    expect(h.at(7.9).has(3)).toBe(true);
    h.at(8);
    expect(h.interp.left).toEqual([3]);
    expect(h.interp.entities.size).toBe(0);
  });

  test("a reused id is a new entity: the old one leaves where it was", () => {
    const h = harness();
    h.frame(1, { full: true, spawn: [{ id: 7, pos: [0, 0, 0], components: { 1: [1] } }] });
    h.frame(3, { despawn: [7], spawn: [{ id: 7, pos: [50, 0, 0] }] });
    const old = h.at(2.5).get(7)!;
    expect(old.x).toBeCloseTo(0);
    expect(old.components.get(1)).toEqual(new Uint8Array([1]));
    const fresh = h.at(3).get(7)!;
    expect(h.interp.left).toEqual([7]);
    expect(h.interp.entered).toEqual([7]);
    expect(fresh).not.toBe(old);
    expect(fresh.x).toBeCloseTo(50);
    expect(fresh.components.size).toBe(0);
  });

  test("a quiet entity stands still until the tick before its next move", () => {
    const h = harness();
    h.frame(1, { full: true, spawn: [{ id: 1, pos: [0, 0, 0] }] });
    h.frame(10, { update: [{ id: 1, from: [0, 0, 0], pos: [10, 0, 0] }] });
    expect(h.at(5).get(1)?.x).toBeCloseTo(0);
    expect(h.at(9.5).get(1)?.x).toBeCloseTo(5);

    const spread = harness({ holdWhenQuiet: false });
    spread.frame(1, { full: true, spawn: [{ id: 1, pos: [0, 0, 0] }] });
    spread.frame(11, { update: [{ id: 1, from: [0, 0, 0], pos: [10, 0, 0] }] });
    expect(spread.at(6).get(1)?.x).toBeCloseTo(5);
  });

  test("a move longer than snapDistance jumps instead of sliding", () => {
    const h = harness({ snapDistance: 5 });
    h.frame(1, { full: true, spawn: [{ id: 1, pos: [0, 0, 0] }] });
    h.frame(2, { update: [{ id: 1, from: [0, 0, 0], pos: [100, 0, 0] }] });
    h.frame(3, { update: [{ id: 1, from: [100, 0, 0], pos: [102, 0, 0] }] });
    expect(h.at(1.9).get(1)?.x).toBeCloseTo(0);
    expect(h.at(2).get(1)?.x).toBeCloseTo(100);
    expect(h.at(2.5).get(1)?.x).toBeCloseTo(101);
  });

  test("components change at the tick they changed, with the positions", () => {
    const h = harness();
    h.frame(1, { full: true, spawn: [{ id: 1, components: { 1: [100] } }] });
    h.frame(3, { update: [{ id: 1, components: { 1: [40], 2: [9] } }] });
    h.frame(4, { update: [{ id: 1, components: { 2: null } }] });
    expect(h.at(2.9).get(1)?.components.get(1)).toEqual(new Uint8Array([100]));
    const e = h.at(3.5).get(1)!;
    expect(e.components.get(1)).toEqual(new Uint8Array([40]));
    expect(e.to.components.has(2)).toBe(false);
    expect(h.at(4).get(1)?.components.has(2)).toBe(false);
  });

  test("a full frame ends entities it lacks and continues the rest", () => {
    const h = harness();
    h.frame(1, { full: true, spawn: [{ id: 1 }, { id: 2, pos: [0, 0, 0] }] });
    h.at(1);
    h.frame(5, { full: true, spawn: [{ id: 2, pos: [4, 0, 0] }] });
    expect(h.at(4.5).has(1)).toBe(true);
    const e = h.at(5).get(2)!;
    expect(h.interp.left).toEqual([1]);
    expect(h.interp.entered).toEqual([]);
    expect(e.x).toBeCloseTo(4);
  });

  test("ticks that go back (a restarted shard) start over", () => {
    const h = harness();
    h.frame(100, { full: true, spawn: [{ id: 1, pos: [5, 0, 0] }] });
    h.at(100);
    h.frame(1, { full: true, spawn: [{ id: 9 }] });
    const now = h.at(1);
    expect([...now.keys()]).toEqual([9]);
    expect(h.interp.left).toEqual([1]);
  });

  test("keeps a bounded history", () => {
    const h = harness({ maxSamples: 4 });
    h.frame(0, { full: true, spawn: [{ id: 1, pos: [0, 0, 0] }] });
    for (let t = 1; t <= 50; t++) {
      h.frame(t, { update: [{ id: 1, from: [t - 1, 0, 0], pos: [t, 0, 0] }] });
    }
    // Only the newest samples remain; an old render tick clamps to them.
    expect(h.at(10).get(1)?.x).toBeCloseTo(47);
    expect(h.at(49.5).get(1)?.x).toBeCloseTo(49.5);
  });
});
