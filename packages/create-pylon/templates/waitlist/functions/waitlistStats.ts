import { query } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { SignupRow, WaitlistStatsResult } from "../lib/stats";

// waitlistStats — the owner's view of the raw signups, INCLUDING emails. This
// is the one function allowed to return PII, so it's gated hard: only the
// configured owner (PYLON_OWNER_EMAIL) gets data; anyone else is denied.
//
// The dashboard calls this with `callFn` and re-fetches whenever the live,
// public WaitlistStat count ticks — so the total / chart / list stay current as
// people join, while the emails themselves never travel over entity sync.
//
// `auth: "user"` means an anonymous caller never reaches the handler; the
// extra owner check is what stops a *different* signed-in user from reading the
// list. Functions bypass the Signup read policy, which is exactly why the gate
// has to live here.
function ymd(iso: string): string {
  // Bucket by calendar day in UTC. `toISOString()` is always YYYY-MM-DDT…Z.
  return new Date(iso).toISOString().slice(0, 10);
}

export default query({
  auth: "user",
  async handler(ctx): Promise<WaitlistStatsResult> {
    // AuthInfo carries the userId, not the email — resolve the caller's email
    // from their own User row (the User self-read policy allows that), then
    // check it against the configured owner. A non-owner gets `authorized:
    // false` and NOTHING else — no count, no emails. (We return a flag instead
    // of throwing: a query has no `ctx.error`, and a bare throw reaches the
    // client as a generic, message-stripped HANDLER_ERROR.)
    const me = await ctx.db.get("User", ctx.auth.userId);
    const email = (me?.email as string | undefined) ?? null;
    if (!emailMatchesOwner(email, ctx.env.PYLON_OWNER_EMAIL)) {
      return { authorized: false };
    }

    // Signup denies all client reads; this returns every row (owner-only), so
    // it goes through the intentional cross-user read surface.
    const rows = (await ctx.db.unsafe.list("Signup")) as unknown as SignupRow[];

    const signups = rows
      .map((r) => ({ id: r.id, email: r.email, createdAt: r.createdAt }))
      .sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1));

    // Daily buckets for the last 30 days, zero-filled so the chart is
    // continuous even on days with no signups.
    const byDay = new Map<string, number>();
    for (const r of rows) {
      const key = ymd(r.createdAt);
      byDay.set(key, (byDay.get(key) ?? 0) + 1);
    }
    const now = new Date();
    const todayKey = now.toISOString().slice(0, 10);
    const daily: { date: string; count: number }[] = [];
    for (let i = 29; i >= 0; i--) {
      const d = new Date(now);
      d.setUTCDate(d.getUTCDate() - i);
      const key = d.toISOString().slice(0, 10);
      daily.push({ date: key, count: byDay.get(key) ?? 0 });
    }

    const sevenDaysAgo = new Date(now);
    sevenDaysAgo.setUTCDate(sevenDaysAgo.getUTCDate() - 7);
    const last7 = rows.filter((r) => new Date(r.createdAt) >= sevenDaysAgo).length;

    return {
      authorized: true,
      total: rows.length,
      today: byDay.get(todayKey) ?? 0,
      last7,
      daily,
      signups,
    };
  },
});
