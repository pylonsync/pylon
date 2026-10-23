// Which routes navigate with no page fetch: a page that exports
// `navigation = "client"`, or an offline route.

import { describe, expect, test } from "bun:test";
import { exportsStaticMetadata } from "./ssr-client-bundler";
import { clientRouteFor, declaresClientNavigation, isOfflinePath } from "./ssr-client-nav";
import { matchRoute } from "./ssr-route-match";

const MANIFEST = {
  routes: {
    "app/leads/page": { path: "/leads", navigation: "client" },
    "app/leads/[id]/page": { path: "/leads/[id]" },
    "app/map/page": { path: "/map" },
    "app/reports/page": { path: "/reports" },
  },
  offline: { routes: ["/map", "/leads/[id]"] },
};

describe("declaresClientNavigation", () => {
  test("finds the export in its usual spellings", () => {
    for (const src of [
      `export const navigation = "client";`,
      `export const navigation = 'client'`,
      "export const navigation = `client`;",
      `export const navigation: "client" | "server" = "client";`,
      `export   const navigation="client" as const;`,
    ]) {
      expect(declaresClientNavigation(src)).toBe(true);
    }
  });

  test("ignores other values and other names", () => {
    for (const src of [
      `export const navigation = "server";`,
      `const navigation = "client";`,
      `export const navigationMode = "client";`,
      `// navigation = "client"`,
      "",
    ]) {
      expect(declaresClientNavigation(src)).toBe(false);
    }
  });
});

describe("clientRouteFor", () => {
  test("a page that opted in", () => {
    expect(clientRouteFor(MANIFEST, "/leads", matchRoute)?.component).toBe("app/leads/page");
  });

  test("an offline route, static or dynamic", () => {
    expect(clientRouteFor(MANIFEST, "/map", matchRoute)?.component).toBe("app/map/page");
    const lead = clientRouteFor(MANIFEST, "/leads/42", matchRoute);
    expect(lead?.component).toBe("app/leads/[id]/page");
    expect(lead?.params).toEqual({ id: "42" });
  });

  test("anything else fetches its page", () => {
    expect(clientRouteFor(MANIFEST, "/reports", matchRoute)).toBeNull();
    expect(clientRouteFor(MANIFEST, "/nowhere", matchRoute)).toBeNull();
    expect(clientRouteFor(null, "/leads", matchRoute)).toBeNull();
    const { offline: _o, ...noOffline } = MANIFEST;
    expect(clientRouteFor(noOffline, "/map", matchRoute)).toBeNull();
  });

  test("isOfflinePath matches route patterns", () => {
    expect(isOfflinePath(MANIFEST, "/leads/7", matchRoute)).toBe(true);
    expect(isOfflinePath(MANIFEST, "/leads", matchRoute)).toBe(false);
  });
});

describe("exportsStaticMetadata", () => {
  test("finds a static metadata export", () => {
    expect(exportsStaticMetadata(`export const metadata = { title: "Leads" };`)).toBe(true);
    expect(exportsStaticMetadata(`const metadata = {}; export { metadata };`)).toBe(true);
    expect(exportsStaticMetadata(`export async function generateMetadata() {}`)).toBe(false);
    expect(exportsStaticMetadata(`const metadata = {};`)).toBe(false);
  });
});
