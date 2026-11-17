// Decides when a pointer, a finger, or the keyboard is aiming at a link.
//
// `<Link>` fetches the destination's page on intent, and each fetch is a
// server render. A cursor crosses links it is not aiming at, and a scroll
// starts with a touch on whatever is under the finger, so neither a bare
// mouseenter nor a bare touchstart is intent:
//   - Hover counts once the cursor has stayed on the link for
//     HOVER_INTENT_MS.
//   - A touch counts once it has stayed down for TOUCH_INTENT_MS with no
//     movement. A touch that moves is a scroll.
//   - A mouse or pen press counts at once.
//   - Focus counts at once.
//
// Kept apart from Link.tsx so the rules are unit-testable with a plain
// EventTarget and injected timers.

/** A cursor crossing a link on its way elsewhere leaves it sooner than this. */
export const HOVER_INTENT_MS = 80;
/** A scroll moves the finger sooner than this; a tap lasts longer. */
export const TOUCH_INTENT_MS = 60;

export interface IntentTimers {
  set(fn: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

const windowTimers: IntentTimers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/**
 * Call `onIntent` each time the user aims at `el`. Returns a function that
 * removes the listeners and cancels a pending timer.
 */
export function watchLinkIntent(
  el: EventTarget,
  onIntent: () => void,
  timers: IntentTimers = windowTimers,
): () => void {
  let pending: unknown = null;
  const disarm = () => {
    if (pending === null) return;
    timers.clear(pending);
    pending = null;
  };
  const arm = (ms: number) => {
    disarm();
    pending = timers.set(() => {
      pending = null;
      onIntent();
    }, ms);
  };
  const now = () => {
    disarm();
    onIntent();
  };

  const onMouseEnter = () => arm(HOVER_INTENT_MS);
  const onTouchStart = () => arm(TOUCH_INTENT_MS);
  // A touch also fires pointerdown, before touchstart. The touch rules above
  // decide for it.
  const onPointerDown = (e: Event) => {
    if ((e as PointerEvent).pointerType === "touch") return;
    now();
  };

  // [event, handler, passive]
  const listeners: Array<[string, (e: Event) => void, boolean]> = [
    ["mouseenter", onMouseEnter, false],
    ["mouseleave", disarm, false],
    ["touchstart", onTouchStart, true],
    ["touchmove", disarm, true],
    ["touchcancel", disarm, false],
    // A press starts ~100 ms before the click: enough to start the fetch.
    ["pointerdown", onPointerDown, false],
    // Focus is the keyboard's version of hover, and a stronger signal: nobody
    // tabs onto a link idly.
    ["focus", now, false],
    // The click fetches the page itself. A timer that fired after it would
    // render the same page a second time.
    ["click", disarm, false],
  ];
  for (const [type, handler, passive] of listeners) {
    el.addEventListener(type, handler, passive ? { passive: true } : undefined);
  }
  return () => {
    disarm();
    for (const [type, handler] of listeners) {
      el.removeEventListener(type, handler);
    }
  };
}
