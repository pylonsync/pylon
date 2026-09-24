/**
 * `buildManifest({ build })`: the production build block reaches the
 * manifest as written, and a value `pylon build` cannot use fails when app.ts
 * runs, not at build time.
 */
import { expect, test } from "bun:test";
import { buildManifest, entity, field } from "./index";

const base = {
  name: "t",
  version: "0",
  entities: [entity("Doc", { title: field.string() })],
  routes: [],
};

test("the build block is copied into the manifest", () => {
  const build = {
    target: ["chrome >= 90", "safari >= 14"],
    polyfill: "usage" as const,
    css: { target: "es2019" },
    server: { external: ["sharp"] },
    include: ["content"],
  };
  expect(buildManifest({ ...base, build }).build).toEqual(build);
});

test("no build block, or an empty one, adds nothing", () => {
  expect("build" in buildManifest(base)).toBe(false);
  expect("build" in buildManifest({ ...base, build: {} })).toBe(false);
});

test("invalid values throw with the field name", () => {
  const bad: Array<[unknown, string]> = [
    [{ target: [] }, "build.target"],
    [{ target: [""] }, "build.target"],
    [{ polyfill: "all" }, "build.polyfill"],
    [{ polyfill: "usage" }, "polyfill needs build.target"],
    [{ css: { target: 5 } }, "build.css.target"],
    [{ sourcemap: "yes" }, "build.sourcemap"],
    [{ server: { bundle: "no" } }, "build.server.bundle"],
    [{ server: { external: [""] } }, "build.server.external"],
    [{ include: "content" }, "build.include"],
  ];
  for (const [build, msg] of bad) {
    expect(() => buildManifest({ ...base, build: build as any })).toThrow(msg);
  }
});
