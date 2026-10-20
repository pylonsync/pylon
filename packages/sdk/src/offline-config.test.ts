/**
 * `buildManifest({ offline })`: defaults are filled, the start URL is always
 * an offline route, and a value the runtime cannot use fails when app.ts runs.
 */
import { expect, test } from "bun:test";
import { buildManifest, entity, field } from "./index";

const base = {
  name: "t",
  version: "0",
  entities: [entity("Doc", { title: field.string() })],
  routes: [],
};

test("no offline block adds nothing", () => {
  expect("offline" in buildManifest(base)).toBe(false);
});

test("defaults: start URL /, standalone, the start URL is the only route", () => {
  expect(buildManifest({ ...base, offline: { name: "Sales" } }).offline).toEqual({
    name: "Sales",
    startUrl: "/",
    display: "standalone",
    routes: ["/"],
  });
});

test("the start URL leads the routes and is not listed twice", () => {
  const offline = buildManifest({
    ...base,
    offline: {
      name: "Sales",
      shortName: "Sales",
      themeColor: "#0b2545",
      icons: [{ src: "/icons/192.png", sizes: "192x192", type: "image/png" }],
      startUrl: "/leads?tab=today",
      routes: ["/map", "/leads", "/leads/[id]"],
    },
  }).offline!;
  expect(offline.routes).toEqual(["/leads", "/map", "/leads/[id]"]);
  expect(offline.startUrl).toBe("/leads?tab=today");
  expect(offline.icons).toEqual([{ src: "/icons/192.png", sizes: "192x192", type: "image/png" }]);
});

test("invalid values throw with the field name", () => {
  const bad: Array<[unknown, string]> = [
    [{ name: "" }, "offline.name"],
    [{ name: "x", startUrl: "leads" }, "offline.startUrl"],
    [{ name: "x", startUrl: "https://evil.example/" }, "offline.startUrl"],
    [{ name: "x", startUrl: "//evil.example/" }, "offline.startUrl"],
    [{ name: "x", display: "window" }, "offline.display"],
    [{ name: "x", routes: ["leads"] }, "offline.routes"],
    [{ name: "x", routes: ["/leads?x=1"] }, "offline.routes"],
    [{ name: "x", icons: [{ src: "https://cdn.example/i.png", sizes: "192x192" }] }, "offline.icons"],
    [{ name: "x", icons: [{ src: "/i.png", sizes: "big" }] }, "offline.icons"],
    [{ name: "x", icons: [{ src: "/i.png", sizes: "192x192", purpose: "round" }] }, "offline.icons"],
    [{ name: "x", themeColor: "" }, "offline.themeColor"],
  ];
  for (const [offline, msg] of bad) {
    expect(() => buildManifest({ ...base, offline: offline as never })).toThrow(msg);
  }
});
