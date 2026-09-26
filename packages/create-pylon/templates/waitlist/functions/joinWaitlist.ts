import { mutation, v } from "@pylonsync/functions";
import { syncWaitlistStat } from "../lib/stats";

// Normalize + sanity-check an email without pulling in a dependency. This is a
// pragmatic "looks like an email" check, not RFC 5322 — the unique index is the
// real integrity guard. We cap the length so a hostile caller can't stuff a
// megabyte string into the row.
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
function normalizeEmail(raw: string): string | null {
  const email = raw.trim().toLowerCase();
  if (email.length < 3 || email.length > 254) return null;
  if (!EMAIL_RE.test(email)) return null;
  return email;
}

// joinWaitlist — the ONLY way a Signup row is ever written. It's a `mutation`
// (not an `action`): mutations get `ctx.db` access and run as one atomic
// transaction. It also rewrites the public WaitlistStat row, and that change
// syncs to every open tab, which is what makes the counter tick up live.
//
// `auth: "public"` because a landing-page visitor has no account. Public
// mutations are still rate-limited at the HTTP layer in production (the
// framework's rate_limit plugin); here we also validate + dedupe so the same
// email can't inflate the count.
//
// PRIVACY: this returns only `{ ok, alreadyJoined }`. It never returns a Signup
// row, an id, or anyone else's email.
export default mutation<{ email: string }, { ok: boolean; alreadyJoined: boolean }>({
  auth: "public",
  args: { email: v.string() },
  async handler(ctx, args) {
    const email = normalizeEmail(args.email);
    if (!email) {
      throw ctx.error("INVALID_ARGS", "Enter a valid email address.");
    }

    // Signup denies ALL client access by policy, so these go through
    // `ctx.db.unsafe` — the explicit "this is an intentional cross-user write
    // from a trusted handler" surface (also future-proofs against
    // PYLON_STRICT_FN_POLICIES). The handler IS the only writer.
    //
    // Dedupe: if this email already joined, report success idempotently rather
    // than erroring — re-submitting the same address is a no-op, not a failure.
    const existing = await ctx.db.unsafe.lookup("Signup", "email", email);
    if (existing) {
      return { ok: true, alreadyJoined: true };
    }

    try {
      await ctx.db.unsafe.insert("Signup", {
        email,
        createdAt: new Date().toISOString(),
      });
    } catch (e) {
      // Lost a race to a concurrent insert of the same email — the unique index
      // on `email` rejected the duplicate. That's still "you're on the list".
      const msg = e instanceof Error ? e.message : String(e);
      if (/unique|constraint|conflict|duplicate/i.test(msg)) {
        return { ok: true, alreadyJoined: true };
      }
      throw e;
    }

    // Keep the public, PII-free WaitlistStat row equal to the Signup count.
    // It is a recount, not +1, so the number cannot drift from the table.
    await syncWaitlistStat(ctx.db.unsafe);

    return { ok: true, alreadyJoined: false };
  },
});
