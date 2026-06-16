// Who owns this newsletter? A newsletter is single-tenant — one business, one
// owner — so ownership is just "the email the owner signs in with", configured
// once via the PYLON_OWNER_EMAIL env var. The owner-only `subscriberStats`
// function reads that env (via `ctx.env`) and compares it here.
//
// Fail closed: if PYLON_OWNER_EMAIL is unset, NOBODY is the owner and the
// dashboard stays locked. That's deliberate — an unset owner on a public site
// must not mean "everyone can read the subscribers". Set it in .env (see
// .env.example) before signing in.

export function normalizeOwner(raw: string | null | undefined): string | null {
  const v = raw?.trim().toLowerCase();
  return v && v.length > 0 ? v : null;
}

// Pure comparator — the caller supplies the configured owner value (from
// `ctx.env.PYLON_OWNER_EMAIL`), so the rule lives in one place and stays
// testable without reaching for the environment here.
export function emailMatchesOwner(
  email: string | null | undefined,
  ownerRaw: string | null | undefined,
): boolean {
  const owner = normalizeOwner(ownerRaw);
  if (!owner) return false;
  return (email ?? "").trim().toLowerCase() === owner;
}
