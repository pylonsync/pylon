// /llms.txt is the first thing an agent reads about Pylon, and often the only
// thing it reads before deciding whether we are relevant. These tests cover the
// content — the part this repo owns. The llmstxt.org serialization is the
// framework's job and is tested there (packages/functions/src/ssr-llms.test.ts).

import { describe, expect, test } from "bun:test";
import llms from "./llms";

const doc = llms();
const details = (Array.isArray(doc.details) ? doc.details : [doc.details ?? ""]).join(" ");
const links = (doc.sections ?? []).flatMap((s) => s.links);

describe("shape", () => {
  test("has the required title and a summary", () => {
    expect(doc.title).toBe("Pylon");
    expect(doc.summary?.length ?? 0).toBeGreaterThan(60);
  });

  test("the prose block carries no markdown headings", () => {
    // A heading in `details` ends the prose block and starts a link section,
    // which would swallow the rest of the document.
    expect(details).not.toMatch(/^#/m);
  });

  test("sections are named and non-empty", () => {
    expect((doc.sections ?? []).length).toBeGreaterThanOrEqual(4);
    for (const section of doc.sections ?? []) {
      expect(section.title).toBeTruthy();
      expect(section.links.length).toBeGreaterThan(0);
    }
  });

  test("has an `Optional` section, so a short-context agent can stop early", () => {
    expect((doc.sections ?? []).map((s) => s.title)).toContain("Optional");
  });
});

describe("guidance", () => {
  test("says when to use Pylon AND when not to", () => {
    expect(details).toContain("When to use Pylon");
    expect(details).toContain("When not to use Pylon");
  });

  test("gives the exact command to start, not a description of starting", () => {
    expect(details).toContain("npm create @pylonsync/pylon@latest");
  });

  test("says the framework needs no account or key", () => {
    expect(details.toLowerCase()).toContain("no key to request");
  });

  test("tells an agent how to read this site programmatically", () => {
    expect(details).toContain("Accept: text/markdown");
  });
});

describe("links", () => {
  const urls = links.map((l) => l.url);

  test("names every machine interface", () => {
    for (const needle of [
      "https://www.pylonsync.com/mcp",
      "https://www.pylonsync.com/mcp.json",
      "https://www.pylonsync.com/openapi.json",
      "https://www.pylonsync.com/pylon-skill.md",
      "https://www.pylonsync.com/sitemap.xml",
      "https://docs.pylonsync.com/llms.txt",
    ]) {
      expect(urls, `missing ${needle}`).toContain(needle);
    }
  });

  test("names the CLI package on npm", () => {
    // "We have a CLI" is not discoverable. The registry URL is.
    expect(urls).toContain("https://www.npmjs.com/package/@pylonsync/cli");
  });

  test("every link has a title and an absolute URL on a host we publish", () => {
    expect(links.length).toBeGreaterThan(15);
    const allowed = [
      "https://www.pylonsync.com",
      "https://docs.pylonsync.com",
      "https://www.usesmallware.com",
      "https://github.com/pylonsync",
      "https://www.npmjs.com/package/@pylonsync",
    ];
    for (const link of links) {
      expect(link.title).toBeTruthy();
      expect(
        allowed.some((prefix) => link.url.startsWith(prefix)),
        `unexpected host: ${link.url}`,
      ).toBe(true);
    }
  });

  test("never uses the apex host, which redirects to www", () => {
    for (const url of urls) expect(url).not.toStartWith("https://pylonsync.com");
  });

  test("no duplicate URLs", () => {
    expect(new Set(urls).size).toBe(urls.length);
  });
});
