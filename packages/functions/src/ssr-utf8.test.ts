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
import {
  buildCloudBadgeChunk,
  streamWithHeadInjection,
} from "./ssr-runtime";

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

async function collect(
  chunks: Uint8Array[],
  headBlob = "",
  bodyBlob = "",
): Promise<string> {
  const out: string[] = [];
  await streamWithHeadInjection(
    readerOf(chunks),
    headBlob,
    (t) => out.push(t),
    bodyBlob,
  );
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

describe("streamWithHeadInjection — body injection", () => {
  const doc = "<html><head><title>t</title></head><body><p>hi</p></body></html>";
  const withBadge =
    "<html><head><title>t</title></head><body><p>hi</p><i>B</i></body></html>";

  test("the blob lands immediately before </body>", async () => {
    const bytes = new TextEncoder().encode(doc);
    expect(await collect([bytes], "", "<i>B</i>")).toBe(withBadge);
  });

  test("head and body blobs both land, in document order", async () => {
    const bytes = new TextEncoder().encode(doc);
    expect(await collect([bytes], "<meta>", "<i>B</i>")).toBe(
      "<html><head><title>t</title><meta></head><body><p>hi</p><i>B</i></body></html>",
    );
  });

  test("</body> split across every chunk boundary still matches", async () => {
    // The carry buffer is the point: React splits wherever it likes, so a
    // marker cut in half must not be missed (dropping the badge) or matched
    // twice (duplicating it).
    const bytes = new TextEncoder().encode(doc);
    for (let at = 1; at < bytes.length; at++) {
      const out = await collect(
        [bytes.slice(0, at), bytes.slice(at)],
        "",
        "<i>B</i>",
      );
      expect(out).toBe(withBadge);
    }
  });

  test("a document with no </body> passes through untouched", async () => {
    // Fragment renders have no body close. Emitting the blob anyway would put
    // a floating badge into markup meant to be embedded.
    const frag = "<p>fragment</p>";
    expect(await collect([new TextEncoder().encode(frag)], "", "<i>B</i>")).toBe(
      frag,
    );
  });

  test("an empty body blob leaves the stream byte-identical", async () => {
    expect(await collect([new TextEncoder().encode(doc)])).toBe(doc);
  });

  test("only the first </body> is injected into", async () => {
    const nested = "<body>a</body><body>b</body>";
    expect(await collect([new TextEncoder().encode(nested)], "", "X")).toBe(
      "<body>aX</body><body>b</body>",
    );
  });
});

describe("buildCloudBadgeChunk", () => {
  test("returns nothing unless the control plane sets the flag", () => {
    // Every paid project and every self-hosted install lands here.
    expect(buildCloudBadgeChunk({})).toBe("");
    expect(buildCloudBadgeChunk({ PYLON_CLOUD_BADGE: "" })).toBe("");
    expect(buildCloudBadgeChunk({ PYLON_CLOUD_BADGE: "0" })).toBe("");
    expect(buildCloudBadgeChunk({ PYLON_CLOUD_BADGE: "false" })).toBe("");
    expect(buildCloudBadgeChunk({ PYLON_CLOUD_BADGE: "no" })).toBe("");
  });

  test("renders one self-contained anchor when enabled", () => {
    const html = buildCloudBadgeChunk({ PYLON_CLOUD_BADGE: "1" });
    expect(html).toContain("stack0.dev");
    expect(html).toContain('aria-label="Built on Stack0"');
    expect(html).toContain('rel="noopener noreferrer"');
    // It renders inside someone else's app: no script, and nothing that costs
    // their visitors a network round trip.
    expect(html).not.toContain("<script");
    expect(html).not.toContain("<link");
    expect(html).not.toContain("@import");
    expect(html.match(/<a /g)?.length).toBe(1);
  });
});
