import { describe, expect, test } from "bun:test";

import { Predictor } from "./prediction";

type Move = { dx: number };
const step = (x: number, m: Move) => x + m.dx;

describe("Predictor", () => {
  test("replays the inputs the shard has not processed on its state", () => {
    const p = new Predictor(step);
    p.push(1, { dx: 1 });
    p.push(2, { dx: 2 });
    p.push(3, { dx: 4 });
    // The shard applied 1 (x = 1) as of this frame.
    expect(p.reconcile(1, 1)).toBe(7);
    expect(p.size).toBe(2);
    // It clamped input 2 (x = 1.5 instead of 3): the prediction follows.
    expect(p.reconcile(1.5, 2)).toBe(5.5);
    expect(p.reconcile(5.5, 3)).toBe(5.5);
    expect(p.size).toBe(0);
  });

  test("forgets refused inputs and inputs that were not sent", () => {
    const p = new Predictor(step);
    p.push(0, { dx: 100 });
    p.push(4, { dx: 1 });
    p.push(5, { dx: 2 });
    p.reject(4);
    p.reject(null);
    expect(p.reconcile(0, 3)).toBe(2);
  });

  test("reset drops everything, and pending inputs are bounded", () => {
    const p = new Predictor(step, { maxPending: 3 });
    for (let seq = 1; seq <= 5; seq++) p.push(seq, { dx: 1 });
    expect(p.size).toBe(3);
    expect(p.reconcile(0, 0)).toBe(3);
    p.reset();
    expect(p.reconcile(10, 0)).toBe(10);
  });
});
