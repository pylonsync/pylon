// Creates the demo teammates the seed spreads records across. Each is a User
// row with a `@demo.invalid` address and no password. Outside dev mode nobody
// can sign in as one: the address can't receive a sign-in or reset email.
// In dev mode (`PYLON_DEV_MODE=true`) the auth API returns sign-in codes
// directly, so anyone can sign in as a demo teammate there.

import type { DbWriter } from "@pylonsync/functions";

export interface DemoPerson {
  name: string;
  email: string;
}

/**
 * The address to try on `attempt` (0, 1, 2, …): the person's own address
 * first, then `local+2@demo.invalid`, `local+3@…`, and so on.
 */
export function demoEmail(email: string, attempt: number): string {
  if (attempt === 0) return email;
  const at = email.indexOf("@");
  return `${email.slice(0, at)}+${attempt + 1}${email.slice(at)}`;
}

/**
 * A User row the seed may reuse: one it created itself on an earlier run.
 * A row with a password or a verified email belongs to someone who
 * registered that address, so the seed never assigns demo records to it.
 */
export function isDemoRow(row: Record<string, unknown>): boolean {
  return !row.passwordHash && !row.emailVerified;
}

/** Returns the id of a demo User row for `person`, creating it if needed. */
export async function ensureDemoUser(db: DbWriter, person: DemoPerson): Promise<string> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const email = demoEmail(person.email, attempt);
    // Users are written by auth, and their policy denies client inserts, so
    // demo rows go through the server-trust surface. The read sees
    // passwordHash, which is how a registered account is told apart.
    const [existing] = await db.unsafe.query("User", { email, $limit: 1 });
    if (!existing) {
      return db.unsafe.insert("User", {
        email,
        displayName: person.name,
        createdAt: new Date(Date.now() - 200 * 86_400_000).toISOString(),
      });
    }
    if (isDemoRow(existing)) return existing.id as string;
  }
  throw new Error(`No free demo address for ${person.email}`);
}
