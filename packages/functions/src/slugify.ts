// ---------------------------------------------------------------------------
// Slug helpers
//
// Multi-tenant Pylon apps almost always have a "named entity with a
// URL slug" pattern — Workspace, Organization, Team, Tenant, Project,
// Channel. The slugification rules are nearly identical across them:
// lowercase, hyphenate, drop punctuation, fall back to a numeric
// suffix on collision. Promoting to the framework so app authors
// don't keep writing it.
// ---------------------------------------------------------------------------

import type { DbReader, DbWriter } from "./types";

/**
 * Pure: turn a human name into a URL-safe base slug. Doesn't check
 * uniqueness — combine with `availableSlug` for that.
 *
 * Rules:
 *   - lowercase
 *   - non-alphanumeric runs become single dashes
 *   - trim leading/trailing dashes
 *   - max 40 chars (so a numeric suffix can land within typical
 *     50-char schema constraints)
 *   - empty result (e.g. emoji-only name) → `fallback`
 *
 * @param name    The user-typed display name to slugify.
 * @param fallback Replacement when the slug strips to empty. Default: "x".
 */
export function slugifyName(name: string, fallback = "x"): string {
  const base = name
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);
  if (base.length >= 2) return base;
  if (base.length === 1) return `${base}1`;
  return fallback;
}

/**
 * Find the first slug that's available in the named entity's `field`
 * column. Tries `base`, then `base-2`, `base-3`, …, then a 6-char
 * random suffix as a final fallback.
 *
 * Reserved slugs (operational subdomain names, brand-protected words)
 * can be passed via `reserved`; matches are skipped.
 *
 * Caller still wraps the eventual insert in try/catch — TOCTOU is
 * always possible with this kind of "check then write" probe, and
 * the framework's UNIQUE index is the actual source of truth. The
 * helper just minimises the number of avoidable retries.
 *
 * @example
 * ```ts
 * const slug = await availableSlug(ctx.db, {
 *   entity: "Workspace",
 *   field: "slug",
 *   name: args.name,
 *   reserved: RESERVED_SUBDOMAINS,
 * });
 * await ctx.db.insert("Workspace", { slug, ... });
 * ```
 */
export async function availableSlug(
  db: DbReader | DbWriter,
  options: {
    entity: string;
    field?: string;
    name: string;
    reserved?: ReadonlySet<string> | readonly string[];
    /** Max numeric suffix to try before falling back to random. Default: 50. */
    maxNumericTries?: number;
  },
): Promise<string> {
  const field = options.field ?? "slug";
  const reserved = toSet(options.reserved);
  const base = slugifyName(options.name);
  const max = options.maxNumericTries ?? 50;

  const candidates: string[] = [base];
  for (let i = 2; i <= max; i++) candidates.push(`${base}-${i}`);

  for (const candidate of candidates) {
    if (reserved.has(candidate)) continue;
    const dup = await db.query(options.entity, { [field]: candidate, $limit: 1 });
    if (dup.length === 0) return candidate;
  }

  // Random suffix fallback. Uses an unambiguous-looking alphabet (no
  // 0/O/1/l) so the slug is readable in URLs.
  const alphabet = "abcdefghjkmnpqrstuvwxyz23456789";
  let rand = "";
  for (let i = 0; i < 6; i++) {
    rand += alphabet[Math.floor(Math.random() * alphabet.length)];
  }
  return `${base}-${rand}`;
}

function toSet(
  input: ReadonlySet<string> | readonly string[] | undefined,
): ReadonlySet<string> {
  if (!input) return new Set<string>();
  if (input instanceof Set) return input;
  return new Set(input);
}
