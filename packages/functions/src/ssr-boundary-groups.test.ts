// The server's boundary walk must agree with the client's on route groups.
//
// Regression: `app/(marketing)/not-found.tsx` served unmatched URLs — the host
// matches those on the manifest's URL path, where a group adds no segment —
// but NOT a notFound() thrown by a page, which walked real directories and so
// never crossed out of the page's own subtree. One class of 404 got the app's
// boundary and the other got the built-in, while ROUTE_PATH_DUPLICATE blocked
// keeping a root copy alongside the group's.
//
// These drive the SERVER walk (findBoundaryIn) over a real fixture tree and
// assert the same answers the client's nearestBoundaryComponent gives for the
// equivalent key set — the two diverging is what caused the bug.

import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

import { findBoundaryIn } from "./ssr-runtime";
import { nearestBoundaryComponent } from "./ssr-client-boundary";

let tmp: string | null = null;

afterEach(() => {
  if (tmp) {
    try {
      fs.rmSync(tmp, { recursive: true, force: true });
    } catch {
      /* best effort */
    }
    tmp = null;
  }
});

/** Create a tree of empty module files and return its root. */
function tree(files: string[]): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-boundary-"));
  for (const f of files) {
    const full = path.join(dir, f);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, "export default function X(){return null}\n", "utf8");
  }
  tmp = dir;
  return dir;
}

/** Module paths (no extension) for the client-side equivalent. */
const keysOf = (files: string[]) => files.map((f) => f.replace(/\.tsx$/, ""));

describe("server boundary walk treats route groups as transparent", () => {
  test("a group's not-found covers a page outside the group", () => {
    const files = ["app/(marketing)/not-found.tsx", "app/gone/page.tsx"];
    const root = tree(files);
    expect(findBoundaryIn(fs, path, root, "app/gone/page", "not-found")).toBe(
      "app/(marketing)/not-found",
    );
    // ...and the client agrees, which is the property that broke.
    expect(
      nearestBoundaryComponent("app/gone/page", "not-found", keysOf(files)),
    ).toBe("app/(marketing)/not-found");
  });

  test("nested groups resolve too", () => {
    const files = ["app/(a)/(b)/not-found.tsx", "app/gone/page.tsx"];
    const root = tree(files);
    expect(findBoundaryIn(fs, path, root, "app/gone/page", "not-found")).toBe(
      "app/(a)/(b)/not-found",
    );
    expect(
      nearestBoundaryComponent("app/gone/page", "not-found", keysOf(files)),
    ).toBe("app/(a)/(b)/not-found");
  });

  test("a real directory's own boundary wins over a group beside it", () => {
    const files = [
      "app/not-found.tsx",
      "app/(marketing)/not-found.tsx",
      "app/gone/page.tsx",
    ];
    const root = tree(files);
    // (An app can't actually ship both — both claim "/" and the duplicate
    // check rejects it — but the walk must still be deterministic.)
    expect(findBoundaryIn(fs, path, root, "app/gone/page", "not-found")).toBe(
      "app/not-found",
    );
  });

  test("a nearer boundary still beats a group one higher up", () => {
    const files = [
      "app/(marketing)/not-found.tsx",
      "app/dashboard/not-found.tsx",
      "app/dashboard/settings/page.tsx",
    ];
    const root = tree(files);
    expect(
      findBoundaryIn(fs, path, root, "app/dashboard/settings/page", "not-found"),
    ).toBe("app/dashboard/not-found");
    expect(
      nearestBoundaryComponent(
        "app/dashboard/settings/page",
        "not-found",
        keysOf(files),
      ),
    ).toBe("app/dashboard/not-found");
  });

  test("a group boundary in a deeper URL scope does not leak upward", () => {
    // app/(marketing)/pricing/not-found is at "/pricing"; a page at
    // "/dashboard" must not get it.
    const files = [
      "app/(marketing)/pricing/not-found.tsx",
      "app/dashboard/page.tsx",
    ];
    const root = tree(files);
    expect(
      findBoundaryIn(fs, path, root, "app/dashboard/page", "not-found"),
    ).toBeNull();
    expect(
      nearestBoundaryComponent("app/dashboard/page", "not-found", keysOf(files)),
    ).toBeNull();
  });

  test("a boundary inside a group still covers that group's own pages", () => {
    const files = [
      "app/(marketing)/not-found.tsx",
      "app/(marketing)/pricing/page.tsx",
    ];
    const root = tree(files);
    expect(
      findBoundaryIn(fs, path, root, "app/(marketing)/pricing/page", "not-found"),
    ).toBe("app/(marketing)/not-found");
  });

  test("error.tsx and loading.tsx follow the same rule", () => {
    const files = [
      "app/(shell)/error.tsx",
      "app/(shell)/loading.tsx",
      "app/reports/page.tsx",
    ];
    const root = tree(files);
    expect(findBoundaryIn(fs, path, root, "app/reports/page", "error")).toBe(
      "app/(shell)/error",
    );
    expect(findBoundaryIn(fs, path, root, "app/reports/page", "loading")).toBe(
      "app/(shell)/loading",
    );
  });

  test("an app with no boundary anywhere still resolves to null", () => {
    const root = tree(["app/page.tsx"]);
    expect(findBoundaryIn(fs, path, root, "app/page", "not-found")).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// Boundary metadata.
//
// Regression: `export const metadata` on not-found.tsx / error.tsx was ignored
// on the catch path (a page calling notFound(), or throwing). The rendered
// head had no <title> at all — and because the framework's built-in 404 body
// supplies one, taking over the boundary LOST the title rather than replacing
// it, leaving the tab showing the raw URL.
// ---------------------------------------------------------------------------

import { renderMetadata } from "./ssr-runtime";

describe("renderMetadata drives boundary heads too", () => {
  test("emits a title element a boundary can hoist", () => {
    const React = require("react");
    const frag = renderMetadata(React, { title: "Missing", description: "d" });
    expect(frag).toBeTruthy();
    const kids = React.Children.toArray(frag.props.children);
    const types = kids.map((k: any) => k.type);
    expect(types).toContain("title");
    expect(types).toContain("meta");
  });

  test("no metadata yields nothing to wrap", () => {
    // The boundary tree must be left alone rather than wrapped in an empty
    // fragment, so hydration matches what the bundler baked for it.
    expect(renderMetadata(require("react"), undefined)).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// A boundary's dynamic og:image URL.
//
// Regression: the URL was composed from the REQUEST path while the file was
// resolved from the boundary's directory. A 404 therefore advertised a card
// under whatever path failed. Usually that 404s in turn — a broken card is
// worse than none, since a crawler now has something to fail on — but at a
// depth where a dynamic route really does define one, it returned 200 and
// served an unrelated page's card. Silent wrong content.
// ---------------------------------------------------------------------------

import { applyAutoSocialImages } from "./ssr-runtime";

describe("boundary og:image points at the boundary's own card", () => {
  // Compare the PATH only. Absolutizing against the request origin needs a
  // trusted host, which is a separate concern from which route the card lives
  // at — and the path is the thing that regressed.
  const ogPath = (m: any) => {
    const u = m?.openGraph?.image as string | undefined;
    if (!u) return u;
    return u.startsWith("http") ? new URL(u).pathname : u;
  };
  const headers = { host: "example.com" };

  test("a root boundary advertises the root card whatever URL failed", () => {
    const root = tree([
      "app/opengraph-image.tsx",
      "app/not-found.tsx",
      "app/[orgSlug]/[eventSlug]/opengraph-image.tsx",
      "app/[orgSlug]/[eventSlug]/page.tsx",
    ]);
    const prev = process.cwd();
    process.chdir(root);
    try {
      for (const url of [
        "/a/b/c/d/e",
        "/nope-does-not-exist",
        // The dangerous one: a real card EXISTS at this depth, so the old
        // behavior returned 200 with another event's image.
        "/no-such-org/no-such-event",
      ]) {
        const md = applyAutoSocialImages("app/not-found", headers, undefined, url, true);
        expect(ogPath(md), `for ${url}`).toBe("/opengraph-image");
      }
    } finally {
      process.chdir(prev);
    }
  });

  test("a page still advertises the card at its own path", () => {
    // The request path IS the route path for a page — including a tail a
    // catch-all consumed, which is why that shortcut exists.
    const root = tree([
      "app/[orgSlug]/[eventSlug]/opengraph-image.tsx",
      "app/[orgSlug]/[eventSlug]/page.tsx",
    ]);
    const prev = process.cwd();
    process.chdir(root);
    try {
      const md = applyAutoSocialImages(
        "app/[orgSlug]/[eventSlug]/page",
        headers,
        undefined,
        "/acme/summit",
      );
      expect(ogPath(md)).toBe("/acme/summit/opengraph-image");
    } finally {
      process.chdir(prev);
    }
  });
});
