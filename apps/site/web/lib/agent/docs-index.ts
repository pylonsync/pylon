// The Pylon documentation index, read from the docs site's own llms.txt.
//
// docs.pylonsync.com already publishes every page with a one-line summary in
// llmstxt.org format, so the index is derived rather than hand-copied — a
// hand-copy of 80 doc links rots within a release. Parsed once per runner and
// held for an hour; a fetch failure falls back to the seed list below, so the
// agent API degrades to "fewer results" instead of "500".

export interface DocEntry {
  /** Page title, as the docs site names it. */
  title: string;
  /** Absolute URL of the human page. */
  url: string;
  /** The one-line summary from llms.txt, when the docs site gives one. */
  summary?: string;
  /** Path under the docs host, e.g. `concepts/entities`. */
  path: string;
}

const DOCS_HOST = "docs.pylonsync.com";
const DOCS_LLMS_TXT = `https://${DOCS_HOST}/llms.txt`;
const TTL_MS = 60 * 60 * 1000;
const FETCH_TIMEOUT_MS = 5_000;

/**
 * Enough of the index to answer the common questions when the docs site is
 * unreachable. Deliberately short: this is a floor, not a copy.
 */
const SEED: DocEntry[] = [
  {
    title: "Introduction",
    path: "introduction",
    url: `https://${DOCS_HOST}/introduction`,
    summary:
      "Pylon is an agent-native full-stack framework with a typed backend, realtime sync, and React SSR in one server.",
  },
  {
    title: "Quickstart",
    path: "quickstart",
    url: `https://${DOCS_HOST}/quickstart`,
    summary:
      "Run Pylon in five minutes with a schema, policy, server function, and live sync.",
  },
  {
    title: "Installation",
    path: "installation",
    url: `https://${DOCS_HOST}/installation`,
    summary: "Install the Pylon CLI and runtime.",
  },
  {
    title: "Entities",
    path: "concepts/entities",
    url: `https://${DOCS_HOST}/concepts/entities`,
    summary: "Declare typed tables with fields, indexes, and relationships.",
  },
  {
    title: "Policies",
    path: "concepts/policies",
    url: `https://${DOCS_HOST}/concepts/policies`,
    summary: "Row-level access rules that live next to your schema.",
  },
  {
    title: "Functions",
    path: "concepts/functions",
    url: `https://${DOCS_HOST}/concepts/functions`,
    summary: "Write server-side RPC with queries, mutations, and actions.",
  },
];

const LINK_LINE = /^-\s*\[([^\]]+)\]\(([^)]+)\)\s*(?::\s*(.*))?$/;

/**
 * Parse an llms.txt document into doc entries.
 *
 * Only the link lines matter here (`- [Title](url): summary`). Exported so the
 * parser is testable without a network call — it is the part that breaks when
 * the docs site changes shape.
 */
export function parseLlmsTxt(text: string, host = DOCS_HOST): DocEntry[] {
  const out: DocEntry[] = [];
  const seen = new Set<string>();
  for (const rawLine of text.split("\n")) {
    const line = rawLine.trim();
    if (!line.startsWith("-")) continue;
    const match = line.match(LINK_LINE);
    if (!match) continue;
    const [, title, href, summary] = match;
    let parsed: URL;
    try {
      parsed = new URL(href);
    } catch {
      continue;
    }
    // Only this docs host. The index feeds a fetch tool, so an off-host link
    // would turn `read_pylon_doc` into an open proxy.
    if (parsed.hostname !== host) continue;
    // llms.txt links point at the `.md` variant; the human page drops it.
    const path = parsed.pathname.replace(/^\//, "").replace(/\.md$/, "");
    if (!path || seen.has(path)) continue;
    seen.add(path);
    out.push({
      title: title.trim(),
      path,
      url: `https://${host}/${path}`,
      summary: summary?.trim() || undefined,
    });
  }
  return out;
}

let cache: { at: number; entries: DocEntry[] } | null = null;

/** The doc index, cached for an hour per runner. Never throws. */
export async function docsIndex(): Promise<DocEntry[]> {
  const now = Date.now();
  if (cache && now - cache.at < TTL_MS) return cache.entries;
  try {
    const res = await fetch(DOCS_LLMS_TXT, {
      headers: { accept: "text/plain" },
      signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
    });
    if (res.ok) {
      const entries = parseLlmsTxt(await res.text());
      if (entries.length > 0) {
        cache = { at: now, entries };
        return entries;
      }
    }
  } catch {
    // Fall through to the seed.
  }
  // Cache the seed too, so a docs outage doesn't retry on every request.
  cache = { at: now, entries: SEED };
  return SEED;
}

/**
 * Rank doc entries against a query.
 *
 * Deliberately a scored substring match rather than a search index: the corpus
 * is ~80 one-line entries, so an index would cost more than the scan, and the
 * ranking that matters is "a title hit beats a summary hit".
 */
export function rankDocs(entries: DocEntry[], query: string, limit = 10): DocEntry[] {
  const terms = query
    .toLowerCase()
    .split(/\s+/)
    .map((t) => t.replace(/[^a-z0-9+.#-]/g, ""))
    .filter(Boolean);
  if (terms.length === 0) return entries.slice(0, limit);
  const scored = entries
    .map((entry) => {
      const title = entry.title.toLowerCase();
      const path = entry.path.toLowerCase();
      const summary = (entry.summary ?? "").toLowerCase();
      let score = 0;
      for (const term of terms) {
        if (title === term) score += 12;
        else if (title.includes(term)) score += 6;
        if (path.includes(term)) score += 4;
        if (summary.includes(term)) score += 2;
      }
      return { entry, score };
    })
    .filter((s) => s.score > 0)
    .sort((a, b) => b.score - a.score || a.entry.path.localeCompare(b.entry.path));
  return scored.slice(0, limit).map((s) => s.entry);
}

/**
 * Fetch one doc page as markdown.
 *
 * `path` must name a page that is IN the index. That check is the security
 * boundary: without it this endpoint is an open proxy that fetches whatever URL
 * an agent hands it, from inside our network.
 */
export async function readDoc(
  path: string,
): Promise<
  { ok: true; path: string; url: string; markdown: string } | { ok: false; error: string }
> {
  const clean = path.replace(/^\//, "").replace(/\.md$/, "");
  const entries = await docsIndex();
  const entry = entries.find((e) => e.path === clean);
  if (!entry) {
    return {
      ok: false,
      error: `Unknown doc path "${clean}". List the available paths with search_pylon_docs.`,
    };
  }
  try {
    const res = await fetch(`https://${DOCS_HOST}/${clean}.md`, {
      headers: { accept: "text/markdown, text/plain" },
      signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
    });
    if (!res.ok) {
      return { ok: false, error: `The docs site answered ${res.status} for "${clean}".` };
    }
    return { ok: true, path: clean, url: entry.url, markdown: await res.text() };
  } catch {
    return { ok: false, error: `Could not reach the docs site for "${clean}".` };
  }
}
