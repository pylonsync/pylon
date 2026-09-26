// How a person is named in the UI. The saved display name wins; without one,
// a readable name is built from the email's local part. tests/names.test.ts
// covers the cases.

/**
 * "dana.reyes+work@acme.com" → "Dana Reyes". The `+tag` suffix and digits
 * are dropped, and `.`, `_`, `-` separate words. Returns "" when nothing
 * readable is left.
 */
export function nameFromEmail(email: string | null | undefined): string {
  const local = (email ?? "").split("@")[0]?.split("+")[0] ?? "";
  return local
    .replace(/\d+/g, " ")
    .split(/[._\-\s]+/)
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1).toLowerCase())
    .join(" ");
}

/** Longest name the UI shows. updateProfile enforces the same limit on save. */
export const MAX_NAME = 60;

/**
 * The display name, else a name built from the email, else the email. A
 * saved name that is itself an email address (the members endpoint falls
 * back to the email when no name is set) counts as no name.
 */
export function personName(p: {
  displayName?: string | null;
  name?: string | null;
  email?: string | null;
}): string {
  const saved = (p.displayName ?? p.name ?? "").trim().slice(0, MAX_NAME);
  if (saved && !saved.includes("@")) return saved;
  return (nameFromEmail(p.email) || (p.email ?? "")).slice(0, MAX_NAME);
}

/** First word of `personName`, for greetings. */
export function firstName(p: {
  displayName?: string | null;
  name?: string | null;
  email?: string | null;
}): string {
  return personName(p).split(/\s+/)[0] ?? "";
}

/** One or two initials for an avatar. */
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  const letters = parts.length === 1 ? parts[0].slice(0, 1) : parts[0][0] + parts[parts.length - 1][0];
  return letters.toUpperCase();
}
