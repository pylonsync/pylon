/**
 * SSR stream decoding — multi-byte UTF-8 across chunk boundaries.
 *
 * React's `renderToReadableStream` splits the byte stream at arbitrary
 * offsets, so any non-ASCII character can land with its bytes divided
 * between two chunks. Decoding each chunk independently turns the orphaned
 * bytes into one U+FFFD apiece: `…` (e2 80 a6) arrives as three replacement
 * characters.
 *
 * The failure is silent and position-dependent — markup added anywhere
 * earlier in the document shifts the boundary — so a page renders correctly
 * for months and corrupts on an unrelated CSS change. React only catches it
 * when the string is hydrated and compared against the client render;
 * server-only text corrupts with no warning at all.
 */
import { describe, expect, test } from "bun:test";
import { streamWithHeadInjection } from "./ssr-runtime";

/** A reader over pre-split byte chunks — the shape React hands us. */
function readerOf(chunks: Uint8Array[]): ReadableStreamDefaultReader<Uint8Array> {
  return new ReadableStream<Uint8Array>({
    start(controller) {
      for (const c of chunks) controller.enqueue(c);
      controller.close();
    },
  }).getReader();
}

/** Split `text`'s utf-8 bytes at `at`, mid-character on purpose. */
function splitAt(text: string, at: number): Uint8Array[] {
  const bytes = new TextEncoder().encode(text);
  return [bytes.slice(0, at), bytes.slice(at)];
}

async function collect(chunks: Uint8Array[], headBlob = ""): Promise<string> {
  const out: string[] = [];
  await streamWithHeadInjection(readerOf(chunks), headBlob, (t) => out.push(t));
  return out.join("");
}

describe("streamWithHeadInjection — UTF-8 across chunk boundaries", () => {
  test("an ellipsis split across chunks survives", async () => {
    const html = "<p>Search speakers by name…</p>";
    const bytes = new TextEncoder().encode(html);
    // "…" is e2 80 a6; cut between its first and second byte.
    const cut = bytes.indexOf(0xe2) + 1;
    expect(await collect(splitAt(html, cut))).toBe(html);
  });

  test("every split point of a 3-byte character round-trips", async () => {
    const html = "<p>a…b</p>";
    const bytes = new TextEncoder().encode(html);
    for (let at = 1; at < bytes.length; at++) {
      expect(await collect(splitAt(html, at))).toBe(html);
    }
  });

  test("4-byte characters (emoji) survive every split", async () => {
    const html = "<p>ship it 🚀 now</p>";
    const bytes = new TextEncoder().encode(html);
    for (let at = 1; at < bytes.length; at++) {
      expect(await collect(splitAt(html, at))).toBe(html);
    }
  });

  test("accented names and CJK survive every split", async () => {
    // Ordinary content, not edge cases: a speaker called José, a Japanese
    // session title, a middot in a byline.
    const html = "<p>José · 日本語のセッション</p>";
    const bytes = new TextEncoder().encode(html);
    for (let at = 1; at < bytes.length; at++) {
      expect(await collect(splitAt(html, at))).toBe(html);
    }
  });

  test("a character split across THREE chunks survives", async () => {
    // One byte per chunk — the decoder must hold state across two reads
    // that each produce nothing.
    const enc = new TextEncoder().encode("…");
    const chunks = [enc.slice(0, 1), enc.slice(1, 2), enc.slice(2, 3)];
    expect(await collect(chunks)).toBe("…");
  });

  test("head injection still lands, with a split character before it", async () => {
    const html = "<html><head><title>é…</title></head><body>x</body></html>";
    const bytes = new TextEncoder().encode(html);
    const cut = bytes.indexOf(0xe2) + 1; // mid-ellipsis, before </head>
    const out = await collect(splitAt(html, cut), "<meta name=x>");
    expect(out).toBe(
      "<html><head><title>é…</title><meta name=x></head><body>x</body></html>",
    );
  });

  test("a stream truncated mid-character does not lose the tail silently", async () => {
    // Genuinely broken input. One replacement character is the honest
    // answer; dropping the bytes without a trace is not.
    const enc = new TextEncoder().encode("ok…");
    const out = await collect([enc.slice(0, enc.length - 1)]);
    expect(out.startsWith("ok")).toBe(true);
    expect(out).toContain("�");
  });

  test("pure ASCII is unaffected", async () => {
    const html = "<p>plain ascii only</p>";
    const bytes = new TextEncoder().encode(html);
    for (let at = 1; at < bytes.length; at++) {
      expect(await collect(splitAt(html, at))).toBe(html);
    }
  });
});
