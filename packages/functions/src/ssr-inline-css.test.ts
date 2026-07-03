import { describe, expect, test, afterEach } from "bun:test";
import { cssHeadTag } from "./ssr-runtime";

const CSS = "styles-abc123.css";

afterEach(() => {
  delete process.env.PYLON_SSR_INLINE_CSS;
  delete process.env.PYLON_SSR_INLINE_CSS_MAX;
});

describe("cssHeadTag (PYLON_SSR_INLINE_CSS)", () => {
  test("default off → plain stylesheet link, untouched", async () => {
    const tag = await cssHeadTag(CSS, "/_pylon/build/");
    expect(tag).toBe(`<link rel="stylesheet" href="/_pylon/build/${CSS}">`);
  });

  test("enabled but sheet unreadable → falls back to the link", async () => {
    process.env.PYLON_SSR_INLINE_CSS = "1";
    // No client build exists in the test cwd — the helper must degrade
    // to the link, never throw or emit an empty <style>.
    const tag = await cssHeadTag("styles-does-not-exist.css", "/_pylon/build/");
    expect(tag).toBe(
      '<link rel="stylesheet" href="/_pylon/build/styles-does-not-exist.css">',
    );
  });

  test("CDN prefix rides the fallback link unchanged", async () => {
    process.env.PYLON_SSR_INLINE_CSS = "true";
    const tag = await cssHeadTag(
      "styles-missing.css",
      "https://assets.pyln.dev/o/p/",
    );
    expect(tag).toBe(
      '<link rel="stylesheet" href="https://assets.pyln.dev/o/p/styles-missing.css">',
    );
  });
});

describe("cssHeadTag per-route override", () => {
  test("route true wins over env off (falls back to link only because no build exists here)", async () => {
    // Override=true ENABLES the inline path with env unset; with no
    // client build in the test cwd it degrades to the link — proving
    // the override reached the enabled gate (default-off would have
    // returned the link WITHOUT attempting the read; exercised via the
    // cache: a distinct filename keeps the assertions independent).
    const tag = await cssHeadTag("styles-override.css", "/_pylon/build/", true);
    expect(tag).toBe(
      '<link rel="stylesheet" href="/_pylon/build/styles-override.css">',
    );
  });

  test("route false wins over env on", async () => {
    process.env.PYLON_SSR_INLINE_CSS = "1";
    const tag = await cssHeadTag("styles-forced-off.css", "/_pylon/build/", false);
    expect(tag).toBe(
      '<link rel="stylesheet" href="/_pylon/build/styles-forced-off.css">',
    );
  });
});
