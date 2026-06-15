/**
 * Render integration test for the SSR client not-found / error boundary.
 *
 * Covers the two regressions that shipped because unit + build tests can't
 * reach RENDER-time behavior (they only check pure functions + that the chunk
 * contains the wiring):
 *   - 0.3.270: a client-thrown notFound() blanked the whole page (the client
 *     runtime had no error boundary at all).
 *   - 0.3.271: the boundary rendered not-found.tsx + its layout chain with a
 *     MINIMAL props object, so an auth-guarding layout (`if (!auth?.user_id)
 *     redirect("/login")`) saw no auth and bounced to /login instead of
 *     showing the not-found page.
 *
 * Drives the REAL `createPylonBoundary` with a real React renderer (happy-dom
 * + react-dom/client). Isolated file because it registers DOM globals.
 *
 * NAMING — this file MUST sort AFTER ssr-client-bundler.test.ts (the only test
 * that runs `Bun.build`). `bun test` runs files in order in one process, and
 * once react-dom is loaded a later `Bun.build` fails to read `scheduler`
 * ("Unexpected reading file"). ssr-hydration.test.ts depends on the same
 * ordering; "ssr-runtime-*" sorts after "ssr-client-bundler". Don't rename to
 * sort before the bundler test.
 */
import { afterEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import React from "react";

import { createPylonBoundary } from "./ssr-client-boundary";

// --- DOM globals react-dom/client touches; saved + restored per file. --------
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

// A client notFound(): same digest @pylonsync/react's notFound() stamps on the
// error it throws (NotFoundError.digest === "PYLON_NOT_FOUND").
class NotFoundError extends Error {
  readonly digest = "PYLON_NOT_FOUND";
  constructor() {
    super("PYLON_NOT_FOUND");
  }
}

// The runtime's buildTree: wrap a Page in its Layout chain (root → leaf).
const buildTree = (Page: any, Layouts: any[], props: any) =>
  Layouts.reduceRight(
    (child: any, L: any) => React.createElement(L, props, child),
    React.createElement(Page, props),
  );

// Mount a tree into a fresh happy-dom root, flush effects, return innerHTML.
// console.error is captured because React logs boundary-caught errors + an
// act() advisory; neither is a failure here.
async function mount(tree: any): Promise<string> {
  const win = new Window({ url: "http://localhost/dashboard" });
  registerDom(win);
  const container = win.document.createElement("div");
  win.document.body.appendChild(container);
  const orig = console.error;
  console.error = () => {};
  try {
    const { createRoot } = await import("react-dom/client");
    const root = createRoot(container as any);
    root.render(tree);
    await new Promise((r) => setTimeout(r, 80));
    return (container as any).innerHTML as string;
  } finally {
    console.error = orig;
  }
}

describe("createPylonBoundary (render integration)", () => {
  test("client-thrown notFound() renders the not-found page WITH the page's props — auth flows through, no /login redirect", async () => {
    const redirects: string[] = [];
    // Mirrors app/dashboard/layout.tsx: redirect to /login when no auth.
    function AuthGuardLayout(props: any) {
      if (!props.auth?.user_id) {
        redirects.push("/login");
        return null;
      }
      return props.children;
    }
    function NotFound(props: any) {
      return React.createElement(
        "div",
        { "data-nf": "1" },
        "NOT FOUND user=" + (props.auth?.user_id ?? "none"),
      );
    }
    function ThrowingPage(): any {
      throw new NotFoundError();
    }

    const { withBoundary } = createPylonBoundary({
      loadManifest: async () => ({ routes: { "app/dashboard/not-found": {} } }),
      loadRouteEntry: async () => ({ Page: NotFound, Layouts: [AuthGuardLayout] }),
      navigate: () => {},
      buildTree,
      getPageProps: () => ({ auth: { user_id: "u1" }, params: {}, url: "/d" }),
      getResetHref: () => "/d",
    });

    const html = await mount(
      withBoundary(buildTree(ThrowingPage, [], {}), "app/dashboard/orgs/[s]/page", 0),
    );

    // Caught the throw + rendered the not-found page (NOT a blank page):
    expect(html).toContain("NOT FOUND");
    // ...with the page's auth in props (the 0.3.271 fix) so the auth-guard
    // layout rendered its children instead of redirecting to /login:
    expect(html).toContain("user=u1");
    expect(redirects).toEqual([]);
  });

  test("negative control: with NO auth in the page props the auth-guard layout redirects — proves the test detects the missing-props bug class", async () => {
    const redirects: string[] = [];
    function AuthGuardLayout(props: any) {
      if (!props.auth?.user_id) {
        redirects.push("/login");
        return null;
      }
      return props.children;
    }
    function NotFound() {
      return React.createElement("div", null, "NOT FOUND");
    }
    function ThrowingPage(): any {
      throw new NotFoundError();
    }

    const { withBoundary } = createPylonBoundary({
      loadManifest: async () => ({ routes: { "app/dashboard/not-found": {} } }),
      loadRouteEntry: async () => ({ Page: NotFound, Layouts: [AuthGuardLayout] }),
      navigate: () => {},
      buildTree,
      // No auth — what the 0.3.270 boundary's minimal props object produced.
      getPageProps: () => ({}),
      getResetHref: () => "/d",
    });

    const html = await mount(
      withBoundary(buildTree(ThrowingPage, [], {}), "app/dashboard/x/page", 0),
    );
    expect(html).not.toContain("NOT FOUND");
    expect(redirects).toContain("/login");
  });

  test("a non-notFound error resolves the error boundary, with a SAFE error projection (message + digest only)", async () => {
    let seenError: any = null;
    function ErrorPage(props: any) {
      seenError = props.error;
      return React.createElement("div", null, "ERR:" + (props.error?.message ?? ""));
    }
    function Boom(): any {
      const e: any = new Error("kaboom");
      e.stack = "secret-stack-do-not-leak";
      throw e;
    }
    const { withBoundary } = createPylonBoundary({
      loadManifest: async () => ({ routes: { "app/error": {} } }),
      loadRouteEntry: async () => ({ Page: ErrorPage, Layouts: [] }),
      navigate: () => {},
      buildTree,
      getPageProps: () => ({ auth: { user_id: "u1" } }),
      getResetHref: () => "/d",
    });
    const html = await mount(withBoundary(buildTree(Boom, [], {}), "app/x/page", 0));
    expect(html).toContain("ERR:kaboom");
    // The raw Error (and its stack) must NEVER reach the boundary props.
    expect(seenError).toEqual({ message: "kaboom", digest: undefined });
    expect(JSON.stringify(seenError)).not.toContain("secret-stack");
  });

  test("no boundary module in the manifest → styled default 404, never a blank page", async () => {
    function ThrowingPage(): any {
      throw new NotFoundError();
    }
    const { withBoundary } = createPylonBoundary({
      loadManifest: async () => ({ routes: { "app/page": {} } }), // no not-found anywhere
      loadRouteEntry: async () => {
        throw new Error("should not be called");
      },
      buildTree,
      navigate: () => {},
      getPageProps: () => ({ auth: { user_id: "u1" } }),
      getResetHref: () => "/d",
    });
    const html = await mount(withBoundary(buildTree(ThrowingPage, [], {}), "app/x/page", 0));
    expect(html).toContain("404 — Not found");
  });
});
