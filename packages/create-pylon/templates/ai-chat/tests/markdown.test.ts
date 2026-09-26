import { describe, expect, test } from "bun:test";
import { inlineText, parseInline, parseMarkdown, safeHref } from "../lib/markdown";
import { highlight } from "../lib/highlight";

describe("parseMarkdown", () => {
  test("headings, paragraphs, and lists", () => {
    const b = parseMarkdown("## Title\n\nSome *text*.\n\n- one\n- two\n\n1. first\n2. second");
    expect(b.map((x) => x.type)).toEqual(["heading", "paragraph", "list", "list"]);
    const ol = b[3];
    expect(ol.type === "list" && ol.ordered && ol.items.length).toBe(2);
  });

  test("fenced code keeps its content and language", () => {
    const b = parseMarkdown("```ts\nconst a = `x`;\n\n// **not bold**\n```\nafter");
    expect(b[0]).toEqual({ type: "code", lang: "ts", code: "const a = `x`;\n\n// **not bold**", closed: true });
    expect(b[1].type).toBe("paragraph");
  });

  test("an unclosed fence runs to the end while streaming", () => {
    const b = parseMarkdown("Here:\n```py\nprint(1)");
    expect(b[1]).toEqual({ type: "code", lang: "py", code: "print(1)", closed: false });
  });

  test("tables with alignment", () => {
    const b = parseMarkdown("| A | B |\n| :-- | --: |\n| 1 | 2 |\n| 3 | 4 |");
    expect(b[0].type).toBe("table");
    if (b[0].type === "table") {
      expect(b[0].align).toEqual(["left", "right"]);
      expect(b[0].rows.length).toBe(2);
    }
  });

  test("nested list items", () => {
    const b = parseMarkdown("- parent\n  - child\n- next");
    expect(b[0].type === "list" && b[0].items[0].sublist?.items.length).toBe(1);
    expect(b[0].type === "list" && b[0].items.length).toBe(2);
  });

  test("block quotes and rules", () => {
    const b = parseMarkdown("> quoted\n\n---");
    expect(b.map((x) => x.type)).toEqual(["quote", "hr"]);
  });
});

describe("inline", () => {
  test("bold, italic, code, strikethrough", () => {
    const n = parseInline("**b** *i* `c` ~~d~~");
    expect(n.map((x) => x.type)).toEqual(["strong", "text", "em", "text", "code", "text", "del"]);
  });

  test("snake_case is not emphasis", () => {
    expect(parseInline("use my_var_name here")).toEqual([{ type: "text", text: "use my_var_name here" }]);
  });

  test("unsafe link schemes render as text", () => {
    const n = parseInline("[click](javascript:alert(1)) and [ok](https://pylonsync.com)");
    expect(n.some((x) => x.type === "link" && x.href.startsWith("javascript"))).toBe(false);
    expect(n.some((x) => x.type === "link" && x.href === "https://pylonsync.com")).toBe(true);
    expect(inlineText(n)).toContain("click");
  });

  test("bare URLs become links without trailing punctuation", () => {
    const n = parseInline("See https://example.com/docs.");
    expect(n[1]).toEqual({ type: "link", href: "https://example.com/docs", children: [{ type: "text", text: "https://example.com/docs" }] });
  });

  test("safeHref", () => {
    expect(safeHref("mailto:a@b.c")).toBe("mailto:a@b.c");
    expect(safeHref("/local")).toBe("/local");
    expect(safeHref("//evil.test")).toBeNull();
    expect(safeHref(" JavaScript:alert(1)")).toBeNull();
    expect(safeHref("data:text/html,x")).toBeNull();
  });
});

describe("highlight", () => {
  test("round-trips the source text", () => {
    const src = 'const x = "a"; // note\nreturn 42;';
    expect(highlight(src, "ts").map((t) => t.text).join("")).toBe(src);
  });
  test("tags keywords, strings, comments, numbers", () => {
    const kinds = new Set(highlight('const x = "a"; // c\n1', "ts").map((t) => t.kind));
    for (const k of ["keyword", "string", "comment", "number"]) expect(kinds.has(k as never)).toBe(true);
  });
  test("unknown languages stay plain", () => {
    expect(highlight("anything", "brainfuck")).toEqual([{ kind: "plain", text: "anything" }]);
  });
});
