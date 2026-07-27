import { describe, expect, test } from "bun:test";
import { moveSelection, searchItems, type SearchItem } from "../lib/search";

const items: SearchItem[] = [
  { id: "1", type: "company", title: "Acme Inc", subtitle: "acme.com", href: "/c" },
  {
    id: "2",
    type: "company",
    title: "Acme Corporation Holdings",
    subtitle: "acmeholdings.com",
    href: "/c",
  },
  {
    id: "3",
    type: "contact",
    title: "Dana Whitfield",
    subtitle: "Northwind Logistics",
    href: "/p",
    keywords: "dana@northwind.co VP Operations",
  },
  { id: "4", type: "deal", title: "Fleet dispatch rollout", href: "/d" },
];

const titles = (q: string) => searchItems(items, q).map((i) => i.title);

describe("searchItems", () => {
  test("an empty query lists everything", () => {
    expect(searchItems(items, "").length).toBe(items.length);
    expect(searchItems(items, "   ").length).toBe(items.length);
  });

  test("prefix beats substring", () => {
    expect(titles("acme")[0]).toBe("Acme Inc");
  });

  test("the shorter title wins a tie", () => {
    // Both start with "Acme"; the more specific record should come first.
    expect(titles("acme")).toEqual(["Acme Inc", "Acme Corporation Holdings"]);
  });

  test("matches a word inside the title", () => {
    expect(titles("dispatch")).toContain("Fleet dispatch rollout");
  });

  test("matches the subtitle and hidden keywords", () => {
    expect(titles("northwind")).toContain("Dana Whitfield");
    expect(titles("dana@northwind.co")).toContain("Dana Whitfield");
    expect(titles("VP")).toContain("Dana Whitfield");
  });

  test("is case-insensitive", () => {
    expect(titles("ACME").length).toBe(2);
  });

  test("no match is empty, not everything", () => {
    expect(titles("zzzz")).toEqual([]);
  });

  test("respects the limit", () => {
    expect(searchItems(items, "", 2)).toHaveLength(2);
  });

  test("a word-boundary hit beats a mid-word one", () => {
    const rows: SearchItem[] = [
      { id: "a", type: "deal", title: "Broadcasting", href: "/" },
      { id: "b", type: "deal", title: "New cast list", href: "/" },
    ];
    expect(searchItems(rows, "cast").map((r) => r.title)[0]).toBe("New cast list");
  });
});

describe("moveSelection", () => {
  test("wraps at both ends", () => {
    expect(moveSelection(0, -1, 3)).toBe(2);
    expect(moveSelection(2, 1, 3)).toBe(0);
    expect(moveSelection(0, 1, 3)).toBe(1);
  });

  test("an empty list stays at zero rather than going negative", () => {
    expect(moveSelection(0, 1, 0)).toBe(0);
    expect(moveSelection(0, -1, 0)).toBe(0);
  });
});
