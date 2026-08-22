// The docs-index parser and ranker. Both are pure, and both are where this
// breaks: the parser when the docs site changes shape, the ranker when a
// summary hit starts outranking a title hit.

import { describe, expect, test } from "bun:test";
import { parseLlmsTxt, rankDocs, type DocEntry } from "./docs-index";

const SAMPLE = `# Pylon

> The agent-native full-stack framework.

## Docs

- [Introduction](https://docs.pylonsync.com/introduction.md): Pylon is an agent-native full-stack framework.
- [Entities](https://docs.pylonsync.com/concepts/entities.md): Declare typed tables with fields and indexes.
- [Policies](https://docs.pylonsync.com/concepts/policies.md)
- [Off-site](https://example.com/evil.md): should never be indexed
- [Relative](/introduction.md): not an absolute URL
- not a link line
`;

describe("parseLlmsTxt", () => {
  test("reads title, path, and summary from the link lines", () => {
    const entries = parseLlmsTxt(SAMPLE);
    expect(entries.map((e) => e.path)).toEqual([
      "introduction",
      "concepts/entities",
      "concepts/policies",
    ]);
    expect(entries[0].title).toBe("Introduction");
    expect(entries[0].summary).toBe("Pylon is an agent-native full-stack framework.");
    // The `.md` suffix belongs to the source, not to the human page.
    expect(entries[0].url).toBe("https://docs.pylonsync.com/introduction");
  });

  test("a link with no summary is kept, without one", () => {
    const entry = parseLlmsTxt(SAMPLE).find((e) => e.path === "concepts/policies");
    expect(entry?.summary).toBeUndefined();
  });

  test("off-host and relative links are dropped", () => {
    // This is the security boundary, not tidiness: the index is the allowlist
    // `read_pylon_doc` fetches from. An off-host entry would make it a proxy.
    const entries = parseLlmsTxt(SAMPLE);
    expect(entries.some((e) => e.url.includes("example.com"))).toBe(false);
    expect(entries.some((e) => e.title === "Relative")).toBe(false);
  });

  test("duplicate paths collapse", () => {
    const entries = parseLlmsTxt(
      `- [A](https://docs.pylonsync.com/x.md)\n- [B](https://docs.pylonsync.com/x.md)\n`,
    );
    expect(entries).toHaveLength(1);
    expect(entries[0].title).toBe("A");
  });

  test("an empty or shapeless document yields nothing rather than throwing", () => {
    expect(parseLlmsTxt("")).toEqual([]);
    expect(parseLlmsTxt("# Just a heading\n\nSome prose.\n")).toEqual([]);
  });
});

describe("rankDocs", () => {
  const entries: DocEntry[] = [
    { title: "Policies", path: "concepts/policies", url: "u1", summary: "Row-level access rules." },
    { title: "Entities", path: "concepts/entities", url: "u2", summary: "Typed tables and policies." },
    { title: "Search", path: "concepts/search", url: "u3", summary: "Full-text search with facets." },
  ];

  test("a title match outranks a summary match", () => {
    const hits = rankDocs(entries, "policies");
    expect(hits[0].path).toBe("concepts/policies");
    expect(hits[1].path).toBe("concepts/entities");
  });

  test("every term has to hit something", () => {
    expect(rankDocs(entries, "zzzz")).toEqual([]);
  });

  test("matching is case- and punctuation-insensitive", () => {
    expect(rankDocs(entries, "  POLICIES!  ")[0].path).toBe("concepts/policies");
  });

  test("an empty query returns the head of the index, not nothing", () => {
    expect(rankDocs(entries, "   ", 2)).toHaveLength(2);
  });

  test("the limit is honored", () => {
    expect(rankDocs(entries, "concepts", 1)).toHaveLength(1);
  });
});
