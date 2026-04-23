"use client";

import * as React from "react";

export default function SkillClient({ content }: { content: string }) {
  const [copied, setCopied] = React.useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(content);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      // ignore
    }
  }

  const lines = content.split("\n");

  return (
    <div className="skill-card">
      <div className="skill-card-head">
        <div className="skill-card-meta">
          <span className="skill-dot" />
          <span className="skill-file">SKILL.md</span>
          <span className="skill-dim">·</span>
          <span className="skill-dim">
            {lines.length.toLocaleString()} lines ·{" "}
            {Math.round(content.length / 1024)} KB
          </span>
        </div>
        <div className="skill-card-actions">
          <a
            href="/pylon-skill.md"
            download="SKILL.md"
            className="skill-btn skill-btn-ghost"
          >
            Download
          </a>
          <button onClick={copy} className="skill-btn skill-btn-primary">
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
      </div>
      <pre className="skill-pre">
        <code>{content}</code>
      </pre>
    </div>
  );
}
