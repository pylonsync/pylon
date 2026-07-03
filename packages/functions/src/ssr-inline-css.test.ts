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
