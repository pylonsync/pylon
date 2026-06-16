import { query } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { InquiryRow, OwnerInquiriesResult } from "../lib/agency";

// inquiriesForOwner — the owner's view of every project lead, INCLUDING the
// prospect's name, email, company, and budget. The one function allowed to
// return that PII, gated to the configured owner (PYLON_OWNER_EMAIL via
// ctx.env). A query has no `ctx.error`, so a non-owner gets `{ authorized:
// false }` (a bare throw would surface as a stripped HANDLER_ERROR) and NO data.
//
// The dashboard calls it with `callFn` and re-fetches whenever the public
// Capacity row changes — so booking a lead (which moves the counter) refreshes
// the pipeline without a page reload, while contact details never travel over
// entity sync.
export default query({
  auth: "user",
  async handler(ctx): Promise<OwnerInquiriesResult> {
    const me = await ctx.db.get("User", ctx.auth.userId);
    const email = (me?.email as string | undefined) ?? null;
    if (!emailMatchesOwner(email, ctx.env.PYLON_OWNER_EMAIL)) {
      return { authorized: false };
    }

    const rows = (await ctx.db.unsafe.list("Inquiry")) as unknown as InquiryRow[];
    const inquiries = rows
      .map((r) => ({ ...r }))
      .sort((a, b) => (a.createdAt < b.createdAt ? 1 : a.createdAt > b.createdAt ? -1 : 0));

    return { authorized: true, inquiries };
  },
});
