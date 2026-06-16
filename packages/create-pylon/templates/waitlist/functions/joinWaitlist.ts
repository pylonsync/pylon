import { mutation, v } from "@pylonsync/functions";

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
// transaction, and the insert fires a change event that re-runs the live
// `waitlistCount` query for every open tab — that's what makes the counter tick
// up in realtime.
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

    // Keep the public, PII-free WaitlistStat singleton in sync with the real
    // count. We RECOUNT (rather than +1) so the number can never drift, and
    // this whole handler is one transaction — on SQLite writers serialize, so
    // the recount-then-write is consistent. The landing page reads this row via
    // `db.useQuery`, which syncs the new value to every open tab.
    const total = (await ctx.db.unsafe.list("Signup")).length;
    const stat = (await ctx.db.unsafe.list("WaitlistStat"))[0] as
      | { id: string }
      | undefined;
    const now = new Date().toISOString();
    if (stat) {
      await ctx.db.unsafe.update("WaitlistStat", stat.id, { count: total, updatedAt: now });
    } else {
      await ctx.db.unsafe.insert("WaitlistStat", { count: total, updatedAt: now });
    }

    return { ok: true, alreadyJoined: false };
  },
});
