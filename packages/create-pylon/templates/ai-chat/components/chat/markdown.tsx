"use client";

import React, { useMemo, useState } from "react";
import { parseMarkdown, type Block, type Inline, type ListItem } from "@/lib/markdown";
import { highlight } from "@/lib/highlight";
import { CheckIcon, CopyIcon } from "./icons";

// Renders a reply's Markdown as React elements. Text is always set as text
// content, so model output cannot inject HTML or scripts; links are limited
// to http(s), mailto, and same-site paths by lib/markdown.ts.
export function Markdown({ text }: { text: string }) {
  const blocks = useMemo(() => parseMarkdown(text), [text]);
  return <div className="prose-reply">{blocks.map((b, i) => renderBlock(b, i))}</div>;
}

function renderBlock(b: Block, key: number): React.ReactNode {
  switch (b.type) {
    case "heading": {
      const Tag = (["h2", "h2", "h3", "h4"] as const)[b.level - 1];
      return <Tag key={key}>{renderInline(b.children)}</Tag>;
    }
    case "paragraph":
      return <p key={key}>{renderInline(b.children)}</p>;
    case "code":
      return <CodeBlock key={key} code={b.code} lang={b.lang} />;
    case "list":
      return renderList(b.ordered, b.start, b.items, key);
    case "quote":
      return <blockquote key={key}>{b.children.map((c, i) => renderBlock(c, i))}</blockquote>;
    case "hr":
      return <hr key={key} />;
    case "table":
      return (
        <div key={key} className="table-wrap">
          <table>
            <thead>
              <tr>
                {b.head.map((cell, i) => (
                  <th key={i} style={{ textAlign: b.align[i] ?? undefined }}>
                    {renderInline(cell)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {b.rows.map((row, r) => (
                <tr key={r}>
                  {row.map((cell, i) => (
                    <td key={i} style={{ textAlign: b.align[i] ?? undefined }}>
                      {renderInline(cell)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
  }
}

function renderList(ordered: boolean, start: number, items: ListItem[], key: number): React.ReactNode {
  const children = items.map((item, i) => (
    <li key={i}>
      {renderInline(item.children)}
      {item.sublist ? renderList(item.sublist.ordered, item.sublist.start, item.sublist.items, 0) : null}
    </li>
  ));
  return ordered ? (
    <ol key={key} start={start !== 1 ? start : undefined}>
      {children}
    </ol>
  ) : (
    <ul key={key}>{children}</ul>
  );
}

function renderInline(nodes: Inline[]): React.ReactNode[] {
  return nodes.map((n, i) => {
    switch (n.type) {
      case "text":
        return n.text.includes("\n")
          ? n.text.split("\n").flatMap((part, j) => (j === 0 ? [part] : [<br key={`${i}-${j}`} />, part]))
          : n.text;
      case "code":
        return <code key={i}>{n.text}</code>;
      case "strong":
        return <strong key={i}>{renderInline(n.children)}</strong>;
      case "em":
        return <em key={i}>{renderInline(n.children)}</em>;
      case "del":
        return <del key={i}>{renderInline(n.children)}</del>;
      case "link":
        return (
          <a key={i} href={n.href} target="_blank" rel="noopener noreferrer nofollow">
            {renderInline(n.children)}
          </a>
        );
    }
  });
}

const LANG_LABELS: Record<string, string> = {
  ts: "TypeScript",
  tsx: "TSX",
  typescript: "TypeScript",
  js: "JavaScript",
  jsx: "JSX",
  javascript: "JavaScript",
  py: "Python",
  python: "Python",
  sql: "SQL",
  sh: "Shell",
  bash: "Bash",
  zsh: "Shell",
  json: "JSON",
  rs: "Rust",
  rust: "Rust",
  go: "Go",
  html: "HTML",
  css: "CSS",
  yaml: "YAML",
  yml: "YAML",
  md: "Markdown",
  env: ".env",
};

export function CodeBlock({ code, lang }: { code: string; lang: string }) {
  const [copied, setCopied] = useState(false);
  const tokens = useMemo(() => highlight(code, lang), [code, lang]);

  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      /* clipboard blocked; the code stays selectable */
    }
  }

  return (
    <div className="code-block">
      <div className="code-block-bar">
        <span>{LANG_LABELS[lang.toLowerCase()] ?? (lang || "Code")}</span>
        <button type="button" onClick={copy} aria-label={copied ? "Copied" : "Copy code"}>
          {copied ? <CheckIcon size={14} /> : <CopyIcon size={14} />}
          <span>{copied ? "Copied" : "Copy"}</span>
        </button>
      </div>
      <pre>
        <code>
          {tokens.map((t, i) =>
            t.kind === "plain" ? t.text : (
              <span key={i} className={`tok-${t.kind}`}>
                {t.text}
              </span>
            ),
          )}
        </code>
      </pre>
    </div>
  );
}
