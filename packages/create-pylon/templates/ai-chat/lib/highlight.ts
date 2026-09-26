// A small syntax highlighter for code blocks in replies. It splits code into
// comment, string, number, keyword, and plain tokens with one pass per line
// set. It does not parse the language; it colors what most languages share.

export type TokenKind = "comment" | "string" | "number" | "keyword" | "fn" | "plain";

export interface Token {
  kind: TokenKind;
  text: string;
}

const KEYWORDS: Record<string, string[]> = {
  js: "const let var function return if else for while do switch case break continue new class extends import export from default async await try catch finally throw typeof instanceof in of this null undefined true false void yield interface type enum implements public private protected readonly as satisfies keyof".split(" "),
  py: "def return if elif else for while in not and or is import from as class try except finally raise with lambda yield pass break continue None True False async await global nonlocal self".split(" "),
  sql: "select from where and or not insert into values update set delete create table index on join left right inner outer group by order having limit offset as distinct null is in exists primary key references unique default begin commit returning with case when then else end".split(" "),
  rust: "fn let mut pub use mod struct enum impl trait for in if else match loop while return self Self crate super where as async await move ref const static true false Some None Ok Err".split(" "),
  go: "func package import var const type struct interface map chan go defer return if else for range switch case default break continue nil true false".split(" "),
  sh: "if then else elif fi for in do done while case esac function return export local echo cd set".split(" "),
};

const ALIASES: Record<string, string> = {
  js: "js", jsx: "js", ts: "js", tsx: "js", javascript: "js", typescript: "js", json: "js",
  py: "py", python: "py",
  sql: "sql", postgres: "sql", postgresql: "sql", sqlite: "sql",
  rs: "rust", rust: "rust",
  go: "go", golang: "go",
  sh: "sh", bash: "sh", shell: "sh", zsh: "sh", console: "sh",
};

function family(lang: string): string | null {
  return ALIASES[lang.toLowerCase()] ?? null;
}

export function highlight(code: string, lang: string): Token[] {
  const fam = family(lang);
  if (!fam) return [{ kind: "plain", text: code }];
  const caseInsensitive = fam === "sql";
  const words = new Set(KEYWORDS[fam].map((w) => (caseInsensitive ? w.toLowerCase() : w)));
  const lineComment = fam === "py" || fam === "sh" ? "#" : fam === "sql" ? "--" : "//";
  const blockComments = fam === "js" || fam === "rust" || fam === "go" || fam === "sql";

  const tokens: Token[] = [];
  let plain = "";
  const push = (kind: TokenKind, text: string) => {
    if (plain) {
      tokens.push({ kind: "plain", text: plain });
      plain = "";
    }
    tokens.push({ kind, text });
  };

  let i = 0;
  while (i < code.length) {
    const rest = code.slice(i);

    if (rest.startsWith(lineComment)) {
      const end = code.indexOf("\n", i);
      const stop = end === -1 ? code.length : end;
      push("comment", code.slice(i, stop));
      i = stop;
      continue;
    }
    if (blockComments && rest.startsWith("/*")) {
      const end = code.indexOf("*/", i + 2);
      const stop = end === -1 ? code.length : end + 2;
      push("comment", code.slice(i, stop));
      i = stop;
      continue;
    }

    const ch = code[i];
    if (ch === '"' || ch === "'" || (ch === "`" && fam === "js")) {
      let j = i + 1;
      while (j < code.length && code[j] !== ch) {
        if (code[j] === "\\") j++;
        if (code[j] === "\n" && ch !== "`") break;
        j++;
      }
      push("string", code.slice(i, Math.min(j + 1, code.length)));
      i = Math.min(j + 1, code.length);
      continue;
    }

    const num = rest.match(/^\d[\d_]*(\.\d+)?/);
    if (num && (i === 0 || !/[\w$]/.test(code[i - 1]))) {
      push("number", num[0]);
      i += num[0].length;
      continue;
    }

    const word = rest.match(/^[A-Za-z_$][\w$]*/);
    if (word) {
      const w = word[0];
      const key = caseInsensitive ? w.toLowerCase() : w;
      if (words.has(key)) push("keyword", w);
      else if (code[i + w.length] === "(") push("fn", w);
      else plain += w;
      i += w.length;
      continue;
    }

    plain += ch;
    i++;
  }
  if (plain) tokens.push({ kind: "plain", text: plain });
  return tokens;
}
