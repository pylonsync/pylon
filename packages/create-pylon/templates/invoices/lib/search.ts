// Client-side search powering the ⌘K palette.
//
// A small team's CRM fits in the synced replica, so search runs locally against
// data already in memory: results appear as you type, with no request per
// keystroke. Swap in `db.useSearch` (Pylon's FTS) when a workspace outgrows
// that — the component boundary doesn't change, only this file.

export interface SearchItem {
  id: string;
  type: "deal" | "company" | "contact";
  title: string;
  /** Second line: the company for a contact, the domain for a company. */
  subtitle?: string;
  href: string;
  /** Extra text matched but not displayed — an email, say. */
  keywords?: string;
}

export interface Scored extends SearchItem {
  score: number;
}

/**
 * Rank items against a query. Prefix matches beat word-boundary matches, which
 * beat substring matches; ties break on the shorter title, so "Acme" ranks
 * above "Acme Corporation Holdings" for the query "acme".
 */
export function searchItems(
  items: SearchItem[],
  query: string,
  limit = 8,
): SearchItem[] {
  const q = query.trim().toLowerCase();
  if (!q) return items.slice(0, limit);

  const scored: Scored[] = [];
  for (const item of items) {
    const score = scoreItem(item, q);
    if (score > 0) scored.push({ ...item, score });
  }
  scored.sort(
    (a, b) => b.score - a.score || a.title.length - b.title.length ||
      a.title.localeCompare(b.title),
  );
  return scored.slice(0, limit);
}

function scoreItem(item: SearchItem, q: string): number {
  const title = item.title.toLowerCase();
  if (title.startsWith(q)) return 100;
  if (wordStart(title, q)) return 80;
  if (title.includes(q)) return 60;

  const haystack = `${item.subtitle ?? ""} ${item.keywords ?? ""}`.toLowerCase();
  if (wordStart(haystack, q)) return 40;
  if (haystack.includes(q)) return 20;
  return 0;
}

function wordStart(haystack: string, needle: string): boolean {
  let from = 0;
  for (;;) {
    const at = haystack.indexOf(needle, from);
    if (at === -1) return false;
    if (at === 0 || /[\s._@/-]/.test(haystack[at - 1])) return true;
    from = at + 1;
  }
}

/** Move through a list with arrow keys, wrapping at both ends. */
export function moveSelection(
  current: number,
  delta: number,
  length: number,
): number {
  if (length === 0) return 0;
  return (current + delta + length) % length;
}
