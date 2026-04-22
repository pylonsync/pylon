import * as React from "react";

export type Lang = "ts" | "tsx" | "js" | "rust" | "rs" | "json" | "sh" | "bash";

type Pattern = { cls: string; re: RegExp };

function patternsFor(lang: Lang): Pattern[] {
  if (lang === "ts" || lang === "tsx" || lang === "js") {
    return [
      { cls: "comment", re: /\/\/[^\n]*/ },
      { cls: "str", re: /"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`/ },
      {
        cls: "kw",
        re: /\b(import|from|export|const|let|var|function|async|await|return|if|else|for|while|new|class|extends|typeof|as|default|type|interface|enum|public|private)\b/,
      },
      {
        cls: "type",
        re: /\b(string|number|boolean|void|Promise|Array|Record|Partial|Pick)\b/,
      },
      { cls: "num", re: /\b\d+(\.\d+)?\b/ },
      { cls: "global", re: /\b(db|statecraft|console|window|Math|JSON|Date)\b/ },
      { cls: "fn", re: /\b([a-zA-Z_][a-zA-Z0-9_]*)(?=\s*\()/ },
    ];
  }
  if (lang === "rust" || lang === "rs") {
    return [
      { cls: "comment", re: /\/\/[^\n]*/ },
      { cls: "str", re: /"(?:[^"\\]|\\.)*"/ },
      {
        cls: "kw",
        re: /\b(fn|let|mut|pub|use|crate|struct|enum|impl|trait|async|await|move|return|if|else|for|while|loop|match|self|Self|as|in|ref|type|where)\b/,
      },
      {
        cls: "type",
        re: /\b(String|Vec|HashMap|Option|Result|Box|Arc|Rc|bool|u8|u16|u32|u64|i32|i64|f32|f64|usize|isize)\b/,
      },
      { cls: "num", re: /\b\d+(\.\d+)?\b/ },
      { cls: "fn", re: /\b([a-zA-Z_][a-zA-Z0-9_]*)(?=\s*[!(])/ },
    ];
  }
  if (lang === "json") {
    return [
      { cls: "str", re: /"(?:[^"\\]|\\.)*"/ },
      { cls: "num", re: /\b\d+(\.\d+)?\b/ },
      { cls: "kw", re: /\b(true|false|null)\b/ },
    ];
  }
  // sh / bash
  return [
    { cls: "comment", re: /#[^\n]*/ },
    { cls: "str", re: /"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/ },
    { cls: "accent", re: /^[$❯>]\s/ },
    {
      cls: "kw",
      re: /\b(cargo|statecraft|npm|pnpm|yarn|git|curl|cd|mkdir|export)\b/,
    },
    { cls: "num", re: /\b\d+\b/ },
  ];
}

export function highlightLine(line: string, lang: Lang): React.ReactNode[] {
  const commentMatch = line.match(/^(\s*)(\/\/.*|#.*)$/);
  if (commentMatch) {
    return [
      commentMatch[1],
      <span key="c" className="comment">
        {commentMatch[2]}
      </span>,
    ];
  }
  if (/^\s*(\*|\/\*|\*\/)/.test(line)) {
    return [
      <span key="c" className="comment">
        {line}
      </span>,
    ];
  }

  const patterns = patternsFor(lang);
  const out: React.ReactNode[] = [];
  let remaining = line;
  let key = 0;
  while (remaining.length > 0) {
    let bestIdx = -1;
    let bestLen = 0;
    let bestCls = "";
    for (const p of patterns) {
      const m = remaining.match(p.re);
      if (m && m.index !== undefined) {
        if (bestIdx === -1 || m.index < bestIdx) {
          bestIdx = m.index;
          bestLen = m[0].length;
          bestCls = p.cls;
        }
      }
    }
    if (bestIdx === -1) {
      out.push(remaining);
      break;
    }
    if (bestIdx > 0) out.push(remaining.slice(0, bestIdx));
    const matched = remaining.slice(bestIdx, bestIdx + bestLen);
    out.push(
      <span key={`k${key++}`} className={bestCls}>
        {matched}
      </span>,
    );
    remaining = remaining.slice(bestIdx + bestLen);
  }
  return out;
}

export function CodeLines({ code, lang }: { code: string; lang: Lang }) {
  return (
    <>
      {code.split("\n").map((ln, i) => {
        const nodes = highlightLine(ln, lang);
        return <div key={i}>{nodes.length ? nodes : "\u00A0"}</div>;
      })}
    </>
  );
}
