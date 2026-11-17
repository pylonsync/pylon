// `<Link>` fetches the destination page on intent, and each fetch is a server
// render. These tests hold the rules that keep a scroll through a list of
// links from asking for one render per link.

import { describe, expect, test } from "bun:test";
import { HOVER_INTENT_MS, TOUCH_INTENT_MS, watchLinkIntent } from "./linkIntent";

function harness() {
  let clock = 0;
  let nextId = 1;
  const scheduled = new Map<number, { at: number; fn: () => void }>();
  const el = new EventTarget();
  let intents = 0;
  const stop = watchLinkIntent(
    el,
    () => {
      intents++;
    },
    {
      set: (fn, ms) => {
        const id = nextId++;
        scheduled.set(id, { at: clock + ms, fn });
        return id;
      },
      clear: (id) => {
        scheduled.delete(id as number);
      },
    },
  );
  return {
    stop,
    intents: () => intents,
    pendingTimers: () => scheduled.size,
    fire: (type: string, props: Record<string, unknown> = {}) => {
      el.dispatchEvent(Object.assign(new Event(type), props));
    },
    advance: (ms: number) => {
      clock += ms;
      for (const [id, t] of [...scheduled]) {
        if (t.at <= clock) {
          scheduled.delete(id);
          t.fn();
        }
      }
    },
  };
}

describe("link intent", () => {
  test("a cursor that stays on the link is intent", () => {
    const h = harness();
    h.fire("mouseenter");
    h.advance(HOVER_INTENT_MS - 1);
    expect(h.intents()).toBe(0);
    h.advance(1);
    expect(h.intents()).toBe(1);
  });

  test("a cursor that crosses the link is not intent", () => {
    const h = harness();
    h.fire("mouseenter");
    h.advance(HOVER_INTENT_MS - 1);
    h.fire("mouseleave");
    h.advance(1000);
    expect(h.intents()).toBe(0);
  });

  test("a list scrolled under a resting cursor asks for nothing", () => {
    const h = harness();
    for (let card = 0; card < 26; card++) {
      h.fire("mouseenter");
      h.advance(30);
      h.fire("mouseleave");
    }
    h.advance(1000);
    expect(h.intents()).toBe(0);
  });

  test("a touch that moves is a scroll, not intent", () => {
    const h = harness();
    h.fire("pointerdown", { pointerType: "touch" });
    h.fire("touchstart");
    h.advance(16);
    h.fire("touchmove");
    h.advance(1000);
    expect(h.intents()).toBe(0);
  });

  test("a touch the browser cancels is not intent", () => {
    const h = harness();
    h.fire("touchstart");
    h.fire("touchcancel");
    h.advance(1000);
    expect(h.intents()).toBe(0);
  });

  test("a touch that stays down is intent", () => {
    const h = harness();
    h.fire("pointerdown", { pointerType: "touch" });
    h.fire("touchstart");
    expect(h.intents()).toBe(0);
    h.advance(TOUCH_INTENT_MS);
    expect(h.intents()).toBe(1);
  });

  test("a mouse press is intent at once", () => {
    const h = harness();
    h.fire("mouseenter");
    h.fire("pointerdown", { pointerType: "mouse" });
    expect(h.intents()).toBe(1);
    // The hover timer the press replaced does not fire a second time.
    h.advance(1000);
    expect(h.intents()).toBe(1);
  });

  test("focus is intent at once", () => {
    const h = harness();
    h.fire("focus");
    expect(h.intents()).toBe(1);
  });

  test("a click cancels a pending timer", () => {
    // A quick tap: the browser sends mouseenter and click together after the
    // finger lifts. The click fetches the page; a timer firing after it would
    // render the page again.
    const h = harness();
    h.fire("touchstart");
    h.advance(TOUCH_INTENT_MS - 1);
    h.fire("mouseenter");
    h.fire("click");
    h.advance(1000);
    expect(h.intents()).toBe(0);
  });

  test("stop cancels a pending timer and removes the listeners", () => {
    const h = harness();
    h.fire("mouseenter");
    h.stop();
    expect(h.pendingTimers()).toBe(0);
    h.fire("focus");
    h.advance(1000);
    expect(h.intents()).toBe(0);
  });
});
