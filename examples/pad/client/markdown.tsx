"use client";

import React from "react";

/**
 * Small, safe markdown renderer: headings, bold/italic, inline code,
 * fenced code blocks, links, lists, blockquotes, hr. Renders straight
 * to React elements — never to raw HTML strings — so pasted <script>
 * tags are just text, with no sanitizer needed.
 */

// Inline pass: `code`, **bold**, *italic*, [text](url).
function renderInline(text: string, keyBase: string): React.ReactNode[] {
  const out: React.ReactNode[] = [];
  const pattern =
    /(`[^`]+`)|(\*\*[^*]+\*\*)|(\*[^*]+\*)|(\[([^\]]+)\]\(([^)\s]+)\))/g;
  let last = 0;
  let i = 0;
  for (const m of text.matchAll(pattern)) {
    const idx = m.index ?? 0;
    if (idx > last) out.push(text.slice(last, idx));
    const key = `${keyBase}-${i++}`;
    if (m[1]) {
      out.push(
        <code
          key={key}
          className="rounded bg-zinc-100 px-1 py-0.5 font-mono text-[0.85em] text-pink-600"
        >
          {m[1].slice(1, -1)}
        </code>,
      );
    } else if (m[2]) {
      out.push(<strong key={key}>{m[2].slice(2, -2)}</strong>);
    } else if (m[3]) {
      out.push(<em key={key}>{m[3].slice(1, -1)}</em>);
    } else if (m[4]) {
      const href = m[6];
      // Only plain http(s) links become anchors; anything else stays text.
      if (/^https?:\/\//.test(href)) {
        out.push(
          <a
            key={key}
            href={href}
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-600 underline underline-offset-2"
          >
            {m[5]}
          </a>,
        );
      } else {
        out.push(m[0]);
      }
    }
    last = idx + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

const UL_RE = /^\s*[-*]\s+(.*)$/;
const OL_RE = /^\s*\d+\.\s+(.*)$/;
const H_RE = /^(#{1,4})\s+(.*)$/;
const HR_RE = /^(-{3,}|\*{3,})$/;

export function Markdown({ source }: { source: string }) {
  const lines = source.split("\n");
  const blocks: React.ReactNode[] = [];
  let i = 0;
  let key = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Fenced code block
    if (line.startsWith("```")) {
      const buf: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) {
        buf.push(lines[i]);
        i++;
      }
      i++; // closing fence
      blocks.push(
        <pre
          key={key++}
          className="overflow-x-auto rounded-lg bg-zinc-900 p-4 text-[13px] leading-relaxed text-zinc-100"
        >
          <code>{buf.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    // Headings
    const h = line.match(H_RE);
    if (h) {
      const level = h[1].length;
      const cls = [
        "mt-6 text-3xl font-bold tracking-tight",
        "mt-6 text-2xl font-semibold tracking-tight",
        "mt-5 text-xl font-semibold",
        "mt-4 text-lg font-semibold",
      ][level - 1];
      const content = renderInline(h[2], `h${key}`);
      blocks.push(
        level === 1 ? (
          <h1 key={key++} className={cls}>
            {content}
          </h1>
        ) : level === 2 ? (
          <h2 key={key++} className={cls}>
            {content}
          </h2>
        ) : level === 3 ? (
          <h3 key={key++} className={cls}>
            {content}
          </h3>
        ) : (
          <h4 key={key++} className={cls}>
            {content}
          </h4>
        ),
      );
      i++;
      continue;
    }

    // Horizontal rule
    if (HR_RE.test(line.trim())) {
      blocks.push(<hr key={key++} className="my-6 border-zinc-200" />);
      i++;
      continue;
    }

    // Blockquote (consecutive > lines)
    if (line.startsWith("> ") || line === ">") {
      const buf: string[] = [];
      while (
        i < lines.length &&
        (lines[i].startsWith("> ") || lines[i] === ">")
      ) {
        buf.push(lines[i].replace(/^>\s?/, ""));
        i++;
      }
      blocks.push(
        <blockquote
          key={key++}
          className="border-l-4 border-zinc-300 pl-4 text-zinc-600 italic"
        >
          {renderInline(buf.join(" "), `q${key}`)}
        </blockquote>,
      );
      continue;
    }

    // Lists (unordered + ordered, consecutive lines)
    if (UL_RE.test(line) || OL_RE.test(line)) {
      const ordered = OL_RE.test(line);
      const re = ordered ? OL_RE : UL_RE;
      const items: React.ReactNode[] = [];
      while (i < lines.length && re.test(lines[i])) {
        const item = lines[i].match(re)![1];
        items.push(
          <li key={`li-${key}-${items.length}`}>
            {renderInline(item, `li${key}-${items.length}`)}
          </li>,
        );
        i++;
      }
      blocks.push(
        ordered ? (
          <ol key={key++} className="ml-6 list-decimal space-y-1">
            {items}
          </ol>
        ) : (
          <ul key={key++} className="ml-6 list-disc space-y-1">
            {items}
          </ul>
        ),
      );
      continue;
    }

    // Blank line
    if (line.trim() === "") {
      i++;
      continue;
    }

    // Paragraph (merge consecutive plain lines)
    const buf: string[] = [line];
    i++;
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].startsWith("```") &&
      !H_RE.test(lines[i]) &&
      !lines[i].startsWith("> ") &&
      !UL_RE.test(lines[i]) &&
      !OL_RE.test(lines[i]) &&
      !HR_RE.test(lines[i].trim())
    ) {
      buf.push(lines[i]);
      i++;
    }
    blocks.push(
      <p key={key++} className="leading-relaxed">
        {renderInline(buf.join(" "), `p${key}`)}
      </p>,
    );
  }

  return <div className="space-y-3">{blocks}</div>;
}
