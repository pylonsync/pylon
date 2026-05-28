// Unit tests for OpQueue. The engine relies on three guarantees:
//   1. ops keyed by string serialize against ops keyed by ANY other
//      string (single FIFO chain).
//   2. concurrent enqueues with the SAME key share the running op's
//      promise (no double-fire).
//   3. once an op settles, its key is free for the next enqueue —
//      including a follow-up triggered from the settled op's `.then()`.

import { describe, expect, test } from "bun:test";

import { OpQueue } from "./op-queue";

describe("OpQueue", () => {
  test("serializes ops across different keys", async () => {
    const q = new OpQueue();
    const order: string[] = [];
    const a = q.enqueue("a", async () => {
      order.push("a:start");
      await new Promise((r) => setTimeout(r, 5));
      order.push("a:end");
    });
    const b = q.enqueue("b", async () => {
      order.push("b:start");
      await new Promise((r) => setTimeout(r, 1));
      order.push("b:end");
    });
    await Promise.all([a, b]);
    expect(order).toEqual(["a:start", "a:end", "b:start", "b:end"]);
  });

  test("coalesces concurrent enqueues under the same key", async () => {
    const q = new OpQueue();
    let calls = 0;
    const fn = async () => {
      calls += 1;
      await new Promise((r) => setTimeout(r, 5));
      return calls;
    };
    const p1 = q.enqueue("x", fn);
    const p2 = q.enqueue("x", fn);
    const p3 = q.enqueue("x", fn);
    const [r1, r2, r3] = await Promise.all([p1, p2, p3]);
    expect(calls).toBe(1);
    expect(r1).toBe(1);
    expect(r2).toBe(1);
    expect(r3).toBe(1);
  });

  test("a settled key can be enqueued again", async () => {
    const q = new OpQueue();
    let calls = 0;
    const fn = async () => {
      calls += 1;
      return calls;
    };
    await q.enqueue("y", fn);
    await q.enqueue("y", fn);
    expect(calls).toBe(2);
  });

  test("rejection propagates to all coalesced callers and frees the key", async () => {
    const q = new OpQueue();
    const err = new Error("boom");
    const p1 = q.enqueue("z", async () => {
      throw err;
    });
    const p2 = q.enqueue("z", async () => {
      throw err;
    });
    await expect(p1).rejects.toThrow("boom");
    await expect(p2).rejects.toThrow("boom");

    // Same key is free again after rejection.
    const ok = await q.enqueue("z", async () => "ok");
    expect(ok).toBe("ok");
  });

  test("epoch increments per op", async () => {
    const q = new OpQueue();
    expect(q.currentEpoch()).toBe(0);
    await q.enqueue("e1", async () => {});
    expect(q.currentEpoch()).toBe(1);
    await q.enqueue("e2", async () => {});
    expect(q.currentEpoch()).toBe(2);
    // Coalesced enqueues share the same op, so epoch only bumps once.
    const a = q.enqueue("e3", async () => {});
    const b = q.enqueue("e3", async () => {});
    await Promise.all([a, b]);
    expect(q.currentEpoch()).toBe(3);
  });
});
