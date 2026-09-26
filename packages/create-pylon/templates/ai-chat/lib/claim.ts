// One-time codes for moving a guest's history to an account. The code is 32
// random bytes as hex; only its SHA-256 hash is stored.

export const CLAIM_TTL_MS = 15 * 60 * 1000;

export function newClaimCode(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

export async function hashClaimCode(code: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(code));
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}
