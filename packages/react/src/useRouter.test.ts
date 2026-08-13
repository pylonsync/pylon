// Contract coverage for the Next-compatible router primitives that back the
// Pylon Cloud frontend migration off Next.js: `notFound()`, `redirect()`, and
// the params snapshot read by `useParams`.
//
// We test the observable contracts directly instead of mounting React (the
// package ships no renderer dep — same approach as useRoom.test.ts), and we
// stub a minimal `globalThis.window` since bun:test has no DOM:
//   - `notFound()` throws a branded error the SSR runtime recognizes by digest
//     (the cross-package handshake in ssr-runtime's `asRouteControl`).
//   - `redirect()` delegates to the client runtime's `navigate(_, {replace})`.

import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { NotFoundError, notFound, redirect } from "./useRouter";

describe("notFound() — branded for the SSR not-found boundary", () => {
  test("throws a NotFoundError", () => {
    expect(() => notFound()).toThrow(NotFoundError);
  });

  test("the thrown error carries the exact digest the runtime keys on", () => {
    // This string is the cross-package contract: ssr-runtime.ts's
    // `asRouteControl` matches `err.digest === "PYLON_NOT_FOUND"`. If this
    // literal drifts on either side, server-render notFound() silently 500s
    // instead of 404ing. Pin it on both sides so a drift breaks a test.
    let caught: unknown;
    try {
      notFound();
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(NotFoundError);
    expect((caught as NotFoundError).digest).toBe("PYLON_NOT_FOUND");
  });

  test("notFound() never returns (control always throws)", () => {
    let reached = false;
    try {
      notFound();
      reached = true;
    } catch {
      /* expected */
    }
    expect(reached).toBe(false);
  });
});

describe("redirect() — client-side replace navigation", () => {
  let calls: Array<{ href: string; opts?: unknown }>;
  const hadWindow = "window" in globalThis;
  // The suite preloads happy-dom, so a real window usually EXISTS here. Put it
  // back rather than leaving the stub in place: bun isolates globals per test
  // file today, so nothing observable depends on this, but a test that
  // replaces a global and never restores it is one isolation change away from
  // breaking every file that mounts.
  const originalWindow = (globalThis as any).window;

  beforeEach(() => {
    calls = [];
    // Stand up a minimal window the module can see, without assuming a DOM.
    (globalThis as any).window = {
      __pylon: {
        prefetch: async () => {},
        navigate: async (href: string, opts?: unknown) => {
          calls.push({ href, opts });
        },
      },
    };
  });

  afterEach(() => {
    if (hadWindow) {
      (globalThis as any).window = originalWindow;
    } else {
      delete (globalThis as any).window;
    }
  });

  test("delegates to __pylon.navigate with replace:true (no history push)", () => {
    redirect("/login");
    expect(calls).toHaveLength(1);
    expect(calls[0].href).toBe("/login");
    expect(calls[0].opts).toEqual({ replace: true });
  });

  test("is a no-op (doesn't throw) when the client runtime isn't ready", () => {
    (globalThis as any).window.__pylon = undefined;
    expect(() => redirect("/login")).not.toThrow();
    expect(calls).toHaveLength(0);
  });
});
