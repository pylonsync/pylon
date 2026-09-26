// Turn a model reply into iMessage bubbles.
//
// Messages.app renders plain text: Markdown syntax shows up as literal
// asterisks and pound signs. `toPlainText` rewrites the common Markdown the
// model produces into plain text. `chunkReply` then splits the text into a few
// bubbles along paragraph boundaries, the way a person texts, and hard-splits
// only a paragraph that is too long for one bubble.

export interface ChunkOptions {
  /** Longest bubble, in characters. */
  maxChars: number;
  /** Most bubbles per reply. Extra paragraphs merge into the last bubble. */
  maxBubbles: number;
  /** Paragraphs shorter than this merge with the next one. */
  minChars: number;
  /** Total reply cap. Longer text is cut at a sentence boundary with an ellipsis. */
  maxTotalChars: number;
}

export const DEFAULT_CHUNK_OPTIONS: ChunkOptions = {
  maxChars: 1200,
  maxBubbles: 4,
  minChars: 40,
  maxTotalChars: 4000,
};

export function toPlainText(markdown: string): string {
  let text = markdown.replace(/\r\n?/g, "\n");
  // Fenced code: keep the code, drop the fences.
  text = text.replace(/```[^\n]*\n([\s\S]*?)```/g, (_m, code: string) => code.trimEnd());
  text = text.replace(/`([^`\n]+)`/g, "$1");
  // Images and links: "[label](url)" → "label (url)"; a bare-url label stays as the url.
  text = text.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (_m, alt: string, url: string) =>
    alt ? `${alt} (${url})` : url,
  );
  text = text.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_m, label: string, url: string) =>
    label === url ? url : `${label} (${url})`,
  );
  // Headings → their text.
  text = text.replace(/^#{1,6}\s+(.*)$/gm, "$1");
  // Bold / italic / strikethrough markers.
  text = text.replace(/(\*\*|__)(?=\S)([\s\S]*?\S)\1/g, "$2");
  text = text.replace(/(^|[\s(])(\*|_)(?=\S)([^*_\n]*?\S)\2(?=[\s).,!?:;]|$)/g, "$1$3");
  text = text.replace(/~~(?=\S)([\s\S]*?\S)~~/g, "$1");
  // Bullets → "• ". Numbered lists already read fine.
  text = text.replace(/^\s*[-*+]\s+/gm, "• ");
  // Blockquotes and horizontal rules.
  text = text.replace(/^\s*>\s?/gm, "");
  text = text.replace(/^\s*([-*_])(\s*\1){2,}\s*$/gm, "");
  // Collapse runs of blank lines.
  text = text.replace(/\n{3,}/g, "\n\n");
  return text.trim();
}

/** Split `text` at the last sentence or word boundary at or before `limit`. */
function splitAt(text: string, limit: number): [string, string] {
  if (text.length <= limit) return [text, ""];
  const window = text.slice(0, limit);
  const candidates = [
    window.lastIndexOf("\n"),
    Math.max(window.lastIndexOf(". "), window.lastIndexOf("! "), window.lastIndexOf("? ")) + 1,
  ];
  let cut = Math.max(...candidates);
  if (cut < limit * 0.5) cut = window.lastIndexOf(" ");
  if (cut < limit * 0.3) {
    cut = limit;
    // Never split a UTF-16 surrogate pair (emoji, many CJK extensions).
    const code = text.charCodeAt(cut - 1);
    if (code >= 0xd800 && code <= 0xdbff) cut -= 1;
  }
  return [text.slice(0, cut).trimEnd(), text.slice(cut).trimStart()];
}

function truncateTotal(text: string, max: number): string {
  if (text.length <= max) return text;
  const [head] = splitAt(text, max - 1);
  return `${head}…`;
}

export function chunkReply(
  reply: string,
  options: Partial<ChunkOptions> = {},
): string[] {
  const opts = { ...DEFAULT_CHUNK_OPTIONS, ...options };
  const plain = truncateTotal(toPlainText(reply), opts.maxTotalChars);
  if (plain === "") return [];

  // Paragraphs, with short ones merged forward so a one-line lead-in does not
  // become its own bubble.
  const paragraphs = plain.split(/\n{2,}/).map((p) => p.trim()).filter(Boolean);
  const merged: string[] = [];
  let carry = "";
  for (const p of paragraphs) {
    const next = carry ? `${carry}\n\n${p}` : p;
    if (next.length < opts.minChars) {
      carry = next;
      continue;
    }
    merged.push(next);
    carry = "";
  }
  if (carry) {
    if (merged.length > 0 && merged[merged.length - 1].length + carry.length + 2 <= opts.maxChars) {
      merged[merged.length - 1] = `${merged[merged.length - 1]}\n\n${carry}`;
    } else {
      merged.push(carry);
    }
  }

  // Hard-split paragraphs that exceed one bubble.
  const pieces: string[] = [];
  for (const p of merged) {
    let rest = p;
    while (rest.length > opts.maxChars) {
      const [head, tail] = splitAt(rest, opts.maxChars);
      pieces.push(head);
      rest = tail;
    }
    if (rest) pieces.push(rest);
  }

  // Cap the bubble count: fold the overflow into bubbles that still have room,
  // starting from the end.
  const bubbles = pieces.slice(0, opts.maxBubbles);
  for (const extra of pieces.slice(opts.maxBubbles)) {
    const last = bubbles.length - 1;
    if (bubbles[last].length + extra.length + 2 <= opts.maxChars) {
      bubbles[last] = `${bubbles[last]}\n\n${extra}`;
    } else {
      bubbles[last] = truncateTotal(`${bubbles[last]}\n\n${extra}`, opts.maxChars);
      break;
    }
  }
  return bubbles;
}
