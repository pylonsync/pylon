import { createHash, timingSafeEqual } from "node:crypto";

// Shared-secret checks for the two inbound doors (the Sendblue webhook and the
// Mac relay). Both compare a secret the caller presents against one in the
// server environment.

/**
 * Constant-time string comparison. Both sides are hashed to 32 bytes first so
 * the comparison takes the same time whatever the lengths are, and a length
 * mismatch is not observable.
 */
export function constantTimeEqual(presented: string, expected: string): boolean {
  const a = createHash("sha256").update(presented, "utf8").digest();
  const b = createHash("sha256").update(expected, "utf8").digest();
  return timingSafeEqual(a, b) && presented.length === expected.length;
}

/** Secrets shorter than this are rejected as configuration, not accepted. */
export const MIN_SECRET_LENGTH = 24;

export type SecretCheck =
  | { ok: true }
  | { ok: false; reason: "not_configured" | "missing" | "invalid" };

/**
 * Check a presented secret against the configured one. An unset or too-short
 * configured secret fails closed with `not_configured`: an empty secret must
 * never mean "accept anything".
 */
export function checkSecret(
  presented: string | null | undefined,
  configured: string | null | undefined,
): SecretCheck {
  const expected = (configured ?? "").trim();
  if (expected.length < MIN_SECRET_LENGTH) return { ok: false, reason: "not_configured" };
  const got = (presented ?? "").trim();
  if (got === "") return { ok: false, reason: "missing" };
  return constantTimeEqual(got, expected) ? { ok: true } : { ok: false, reason: "invalid" };
}

/** Pull the token out of an `Authorization: Bearer <token>` header value. */
export function bearerToken(header: string | null | undefined): string | null {
  if (typeof header !== "string") return null;
  const m = header.match(/^\s*Bearer\s+(\S+)\s*$/i);
  return m ? m[1] : null;
}

/** Case-insensitive header read. The runtime lowercases names; this also covers fixtures. */
export function headerValue(
  headers: Record<string, string> | null | undefined,
  name: string,
): string | null {
  if (!headers) return null;
  const want = name.toLowerCase();
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === want) return v;
  }
  return null;
}
