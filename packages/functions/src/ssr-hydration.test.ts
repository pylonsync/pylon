/**
 * #278 Stage 2 — streaming-hydration regression guard (the prod-killer check).
 *
 * Proves the load-bearing claim behind multi-boundary streaming: hydrating the
 * RESOLVED SSR DOM (which is what a streamed page's DOM becomes after React's
 * $RC reveals run) with a `fulfilledThenable`-backed serverData shim produces
 * clean output — the inner <Suspense> boundary renders its RESOLVED rows on the
 * first committed pass, NOT a fallback, and React logs NO hydration mismatch.
 *
 * This is why Pylon needs NO inline per-boundary patch scripts: it withholds
 * the bootstrap from renderToReadableStream and runs hydrateRoot ONCE, after
 * the full ssrData blob — so `use()` reads a fulfilled value synchronously and
 * never re-suspends. If a future change made `use()` suspend at hydration (a
 * broken shim handing back a pending promise), the boundary would mismatch /
 * lose its content — caught here.
 *
 * Isolated in its own file because it registers DOM globals (window/document)
 * for react-dom/client; they're restored after each test.
 */
import { afterEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import React, { Suspense, use } from "react";
import { renderToReadableStream } from "react-dom/server.browser";

// Mirrors `fulfilledThenable` in ssr-client-bundler.ts's CLIENT_RUNTIME_SOURCE:
// a React-recognized fulfilled thenable so use() reads `.value` synchronously
// (status === "fulfilled") instead of suspending.
function fulfilledThenable<T>(value: T) {
  return {
    status: "fulfilled" as const,
    value,
    then(onFulfilled?: (v: T) => any) {
      return onFulfilled ? onFulfilled(value) : (value as any);
    },
  };
}

function Rows({ p }: { p: PromiseLike<string[]> }) {
  const rows = use(p as any) as string[];
  return React.createElement(
    "ul",
    { id: "rows" },
    rows.map((r) => React.createElement("li", { key: r }, r)),
  );
}
function Page({ p }: { p: PromiseLike<string[]> }) {
  return React.createElement(
    "div",
    { id: "app" },
    React.createElement("h1", { id: "shell" }, "Shell"),
    React.createElement(
      Suspense,
      { fallback: React.createElement("p", { id: "fallback" }, "Loading…") },
      React.createElement(Rows, { p }),
    ),
  );
}

const dec = new TextDecoder();

// Server-render the page (data pre-resolved) to its final resolved HTML — the
// same DOM a streamed page settles to once React's reveal scripts have run.
async function renderResolvedHtml(rows: string[]): Promise<string> {
  const stream = await renderToReadableStream(
    React.createElement(Page, { p: Promise.resolve(rows) }),
  );
  await (stream as any).allReady;
  const reader = stream.getReader();
  let html = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    html += dec.decode(value);
  }
  return html;
}

// Globals react-dom/client touches; saved + restored so this file's DOM
// registration never bleeds into sibling test files.
const DOM_GLOBAL_KEYS = [
  "window",
  "document",
  "navigator",
  "HTMLElement",
  "Node",
  "Event",
  "MutationObserver",
  "requestAnimationFrame",
  "cancelAnimationFrame",
];
let savedGlobals: Record<string, any> | null = null;

function registerDom(win: any) {
  const g = globalThis as any;
  savedGlobals = {};
  for (const k of DOM_GLOBAL_KEYS) savedGlobals[k] = g[k];
  g.window = win;
  g.document = win.document;
  g.navigator = win.navigator;
  g.HTMLElement = win.HTMLElement;
  g.Node = win.Node;
  g.Event = win.Event;
  g.MutationObserver = win.MutationObserver;
  g.requestAnimationFrame = (cb: any) => setTimeout(() => cb(Date.now()), 0);
  g.cancelAnimationFrame = (id: any) => clearTimeout(id);
}

afterEach(() => {
  if (!savedGlobals) return;
  const g = globalThis as any;
  for (const k of DOM_GLOBAL_KEYS) {
    if (savedGlobals[k] === undefined) delete g[k];
    else g[k] = savedGlobals[k];
  }
  savedGlobals = null;
});

describe("streaming hydration (#278)", () => {
  test("fulfilledThenable shim → boundary hydrates to RESOLVED rows, no fallback, no mismatch", async () => {
    const rows = ["alpha", "beta"];
    const serverHtml = await renderResolvedHtml(rows);
    // Sanity: the SSR DOM has the resolved rows (not a fallback).
    expect(serverHtml).toContain("<li>alpha</li>");

    const win = new Window({ url: "http://localhost/" });
    // Parse the server HTML into a container element via HTML parsing (this
    // preserves React's <!--$--> boundary comment markers, which hydrateRoot
    // reads to locate Suspense boundaries). insertAdjacentHTML is the parse
    // entry point; the input is our own rendered markup, not untrusted data.
    const root = win.document.createElement("div");
    root.setAttribute("id", "root");
    root.insertAdjacentHTML("afterbegin", serverHtml);
    win.document.body.appendChild(root);
    registerDom(win);

    const errors: string[] = [];
    const origErr = console.error;
    console.error = (...a: any[]) => {
      errors.push(a.map(String).join(" "));
    };
    try {
      const { hydrateRoot } = await import("react-dom/client");
      // Client tree: same component, but serverData yields a fulfilled thenable
      // (the value the server already streamed) — exactly the client shim's job.
      hydrateRoot(
        root as any,
        React.createElement(Page, { p: fulfilledThenable(rows) }),
      );
      await new Promise((r) => setTimeout(r, 50));
      const html = (root as any).innerHTML as string;

      // The boundary committed its RESOLVED content on hydration...
      expect(html).toContain("alpha");
      expect(html).toContain("beta");
      // ...NOT the fallback (no flash / no stuck boundary)...
      expect(html).not.toContain("Loading…");
      // ...and React logged NO hydration mismatch.
      const mismatch = errors.filter((e) =>
        /hydrat|did not match|mismatch|Text content does not match/i.test(e),
      );
      expect(mismatch).toEqual([]);
    } finally {
      console.error = origErr;
    }
  });
});
