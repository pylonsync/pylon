import { describe, expect, test } from "bun:test";
import { chunkReply, toPlainText } from "../lib/chunk";

describe("toPlainText", () => {
  test("strips Markdown the model tends to produce", () => {
    expect(toPlainText("## Plan\n**Bold** and *italic* and `code`")).toBe("Plan\nBold and italic and code");
    expect(toPlainText("- one\n- two\n* three")).toBe("• one\n• two\n• three");
    expect(toPlainText("See [the docs](https://example.com/x)")).toBe("See the docs (https://example.com/x)");
    expect(toPlainText("```ts\nconst a = 1;\n```")).toBe("const a = 1;");
    expect(toPlainText("> quoted\n\n\n\nnext")).toBe("quoted\n\nnext");
  });

  test("leaves ordinary punctuation alone", () => {
    expect(toPlainText("2 * 3 = 6, and a_b_c stays")).toBe("2 * 3 = 6, and a_b_c stays");
  });
});

describe("chunkReply", () => {
  test("a short reply is one bubble", () => {
    expect(chunkReply("Sounds good, see you at 7.")).toEqual(["Sounds good, see you at 7."]);
  });

  test("paragraphs become bubbles; short lead-ins merge forward", () => {
    const para = (n: number) => `Paragraph ${n} has enough words in it to stand alone as a bubble.`;
    expect(chunkReply(`Sure!\n\n${para(1)}\n\n${para(2)}`)).toEqual([`Sure!\n\n${para(1)}`, para(2)]);
  });

  test("caps the bubble count", () => {
    const text = Array.from({ length: 8 }, (_, i) => `This is paragraph number ${i} with some length.`).join("\n\n");
    const bubbles = chunkReply(text, { maxBubbles: 3 });
    expect(bubbles.length).toBe(3);
    expect(bubbles.join("\n\n")).toContain("paragraph number 7");
  });

  test("hard-splits a long paragraph at sentence boundaries", () => {
    const sentence = "This sentence is exactly the kind of thing a model writes. ";
    const bubbles = chunkReply(sentence.repeat(40), { maxChars: 300, maxBubbles: 10, maxTotalChars: 10_000 });
    expect(bubbles.length).toBeGreaterThan(1);
    for (const b of bubbles) {
      expect(b.length).toBeLessThanOrEqual(300);
      expect(b.endsWith(".")).toBe(true);
    }
  });

  test("caps total length with an ellipsis", () => {
    const bubbles = chunkReply("word ".repeat(2000), { maxTotalChars: 500 });
    const total = bubbles.join("").length;
    expect(total).toBeLessThanOrEqual(500);
    expect(bubbles[bubbles.length - 1].endsWith("…")).toBe(true);
  });

  test("never splits an emoji", () => {
    const bubbles = chunkReply("😀".repeat(400), { maxChars: 101, maxBubbles: 20, maxTotalChars: 10_000 });
    for (const b of bubbles) expect(b).not.toMatch(/[\uD800-\uDBFF]$/);
    expect(bubbles.join("")).toBe("😀".repeat(400));
  });

  test("empty input produces no bubbles", () => {
    expect(chunkReply("   ")).toEqual([]);
  });
});
