// A small Markdown parser for assistant replies. It returns a plain tree that
// components/chat/markdown.tsx renders as React elements, so model output is
// never injected as HTML. It covers what models emit: headings, paragraphs,
// bullet and numbered lists (one nesting level), block quotes, fenced code,
// tables, horizontal rules, and inline code, bold, italic, strikethrough, and
// links. An unclosed code fence runs to the end of the text, which keeps a
// reply readable while it streams.

export type Inline =
  | { type: "text"; text: string }
  | { type: "code"; text: string }
  | { type: "strong"; children: Inline[] }
  | { type: "em"; children: Inline[] }
  | { type: "del"; children: Inline[] }
  | { type: "link"; href: string; children: Inline[] };

export interface ListItem {
  children: Inline[];
  sublist?: { ordered: boolean; start: number; items: ListItem[] };
}

export type Block =
  | { type: "heading"; level: 1 | 2 | 3 | 4; children: Inline[] }
  | { type: "paragraph"; children: Inline[] }
  | { type: "code"; lang: string; code: string; closed: boolean }
  | { type: "list"; ordered: boolean; start: number; items: ListItem[] }
  | { type: "quote"; children: Block[] }
  | { type: "table"; align: ("left" | "center" | "right" | null)[]; head: Inline[][]; rows: Inline[][][] }
  | { type: "hr" };

const FENCE = /^\s{0,3}(`{3,}|~{3,})\s*([\w+#.-]*)/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const BULLET = /^(\s*)[-*+]\s+(.*)$/;
const ORDERED = /^(\s*)(\d{1,9})[.)]\s+(.*)$/;
const HR = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const TABLE_SEP = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

function isBlockStart(line: string): boolean {
  return (
    FENCE.test(line) ||
    HEADING.test(line) ||
    BULLET.test(line) ||
    ORDERED.test(line) ||
    HR.test(line) ||
    QUOTE.test(line)
  );
}

function splitRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|") && !s.endsWith("\\|")) s = s.slice(0, -1);
  return s.split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, "|"));
}

export function parseMarkdown(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (line.trim() === "") {
      i++;
      continue;
    }

    const fence = line.match(FENCE);
    if (fence) {
      const marker = fence[1];
      const code: string[] = [];
      let closed = false;
      i++;
      while (i < lines.length) {
        const l = lines[i];
        if (l.trim().startsWith(marker[0].repeat(marker.length)) && l.trim().replace(/[`~]/g, "") === "") {
          closed = true;
          i++;
          break;
        }
        code.push(l);
        i++;
      }
      blocks.push({ type: "code", lang: fence[2] ?? "", code: code.join("\n"), closed });
      continue;
    }

    const heading = line.match(HEADING);
    if (heading) {
      const level = Math.min(heading[1].length, 4) as 1 | 2 | 3 | 4;
      blocks.push({ type: "heading", level, children: parseInline(heading[2]) });
      i++;
      continue;
    }

    if (HR.test(line)) {
      blocks.push({ type: "hr" });
      i++;
      continue;
    }

    if (QUOTE.test(line)) {
      const inner: string[] = [];
      while (i < lines.length && QUOTE.test(lines[i])) {
        inner.push(lines[i].match(QUOTE)![1]);
        i++;
      }
      blocks.push({ type: "quote", children: parseMarkdown(inner.join("\n")) });
      continue;
    }

    if (BULLET.test(line) || ORDERED.test(line)) {
      const { block, next } = parseList(lines, i);
      blocks.push(block);
      i = next;
      continue;
    }

    if (line.includes("|") && i + 1 < lines.length && TABLE_SEP.test(lines[i + 1])) {
      const head = splitRow(line);
      const align = splitRow(lines[i + 1]).map((c) => {
        const left = c.startsWith(":");
        const right = c.endsWith(":");
        return left && right ? "center" : right ? "right" : left ? "left" : null;
      });
      i += 2;
      const rows: Inline[][][] = [];
      while (i < lines.length && lines[i].includes("|") && lines[i].trim() !== "") {
        const cells = splitRow(lines[i]);
        rows.push(head.map((_, c) => parseInline(cells[c] ?? "")));
        i++;
      }
      blocks.push({ type: "table", align, head: head.map(parseInline), rows });
      continue;
    }

    const para: string[] = [];
    while (i < lines.length && lines[i].trim() !== "" && !(para.length > 0 && isBlockStart(lines[i]))) {
      para.push(lines[i].trim());
      i++;
    }
    blocks.push({ type: "paragraph", children: parseInline(para.join("\n")) });
  }

  return blocks;
}

function parseList(lines: string[], start: number): { block: Block; next: number } {
  const first = lines[start];
  const ordered = ORDERED.test(first) && !BULLET.test(first);
  const baseIndent = (first.match(/^(\s*)/)?.[1] ?? "").length;
  const startNum = ordered ? Number(first.match(ORDERED)![2]) : 1;
  const items: ListItem[] = [];
  let i = start;

  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") {
      // A blank line inside a list continues it only when the next line is
      // another item at the same level or deeper.
      const nxt = lines[i + 1];
      if (nxt !== undefined && (BULLET.test(nxt) || ORDERED.test(nxt))) {
        i++;
        continue;
      }
      break;
    }
    const m = ordered ? line.match(ORDERED) : line.match(BULLET);
    const other = ordered ? line.match(BULLET) : line.match(ORDERED);
    const indent = (line.match(/^(\s*)/)?.[1] ?? "").length;

    if (indent > baseIndent && (m || other) && items.length > 0) {
      const { block, next } = parseList(lines, i);
      if (block.type === "list") {
        items[items.length - 1].sublist = { ordered: block.ordered, start: block.start, items: block.items };
      }
      i = next;
      continue;
    }
    if (indent < baseIndent) break;
    if (m && indent === baseIndent) {
      const text = ordered ? m[3] : m[2];
      items.push({ children: parseInline(text) });
      i++;
      continue;
    }
    if (other && indent === baseIndent) break;
    if (isBlockStart(line) && indent <= baseIndent) break;
    // Lazy continuation of the previous item's text.
    if (items.length > 0) {
      items[items.length - 1].children.push({ type: "text", text: "\n" }, ...parseInline(line.trim()));
    }
    i++;
  }
  return { block: { type: "list", ordered, start: startNum, items }, next: i };
}

/* -------------------------------- inline -------------------------------- */

// Links render only for these schemes. Anything else (javascript:, data:,
// vbscript:) renders as plain text.
export function safeHref(href: string): string | null {
  const h = href.trim();
  if (/^(https?:|mailto:)/i.test(h)) return h;
  if (h.startsWith("/") && !h.startsWith("//")) return h;
  if (h.startsWith("#")) return h;
  return null;
}

export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  let buf = "";
  let i = 0;

  const flush = () => {
    if (buf) {
      out.push({ type: "text", text: buf });
      buf = "";
    }
  };

  while (i < text.length) {
    const ch = text[i];

    if (ch === "\\" && i + 1 < text.length && /[\\`*_~[\]()#|>-]/.test(text[i + 1])) {
      buf += text[i + 1];
      i += 2;
      continue;
    }

    if (ch === "`") {
      let ticks = 1;
      while (text[i + ticks] === "`") ticks++;
      const marker = "`".repeat(ticks);
      const end = text.indexOf(marker, i + ticks);
      if (end !== -1) {
        flush();
        out.push({ type: "code", text: text.slice(i + ticks, end).replace(/^ (.*) $/, "$1") });
        i = end + ticks;
        continue;
      }
    }

    if ((ch === "*" || ch === "_") && text[i + 1] === ch) {
      const marker = ch + ch;
      const end = text.indexOf(marker, i + 2);
      if (end > i + 2) {
        flush();
        out.push({ type: "strong", children: parseInline(text.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    }

    if (ch === "~" && text[i + 1] === "~") {
      const end = text.indexOf("~~", i + 2);
      if (end > i + 2) {
        flush();
        out.push({ type: "del", children: parseInline(text.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    }

    if ((ch === "*" || ch === "_") && text[i + 1] !== " " && text[i + 1] !== undefined) {
      // `_` only opens emphasis at a word boundary, so snake_case stays text.
      const boundary = ch === "*" || i === 0 || /[\s(]/.test(text[i - 1]);
      let end = text.indexOf(ch, i + 1);
      while (end !== -1 && text[end + 1] === ch) end = text.indexOf(ch, end + 2);
      if (boundary && end > i + 1 && text[end - 1] !== " ") {
        flush();
        out.push({ type: "em", children: parseInline(text.slice(i + 1, end)) });
        i = end + 1;
        continue;
      }
    }

    if (ch === "[") {
      const close = text.indexOf("]", i + 1);
      if (close !== -1 && text[close + 1] === "(") {
        const end = text.indexOf(")", close + 2);
        if (end !== -1) {
          const label = text.slice(i + 1, close);
          const href = safeHref(text.slice(close + 2, end));
          flush();
          if (href) out.push({ type: "link", href, children: parseInline(label) });
          else out.push(...parseInline(label));
          i = end + 1;
          continue;
        }
      }
    }

    if ((ch === "h" || ch === "H") && /^https?:\/\/[^\s<]+/i.test(text.slice(i)) && (i === 0 || /[\s(]/.test(text[i - 1]))) {
      const m = text.slice(i).match(/^https?:\/\/[^\s<]+/i)!;
      let url = m[0];
      while (/[.,;:!?)]$/.test(url)) url = url.slice(0, -1);
      flush();
      out.push({ type: "link", href: url, children: [{ type: "text", text: url }] });
      i += url.length;
      continue;
    }

    buf += ch;
    i++;
  }
  flush();
  return out;
}

export function inlineText(nodes: Inline[]): string {
  return nodes
    .map((n) => (n.type === "text" || n.type === "code" ? n.text : inlineText(n.children)))
    .join("");
}
