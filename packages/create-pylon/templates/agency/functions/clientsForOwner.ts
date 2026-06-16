import { query } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { ClientRow, OwnerClientsResult } from "../lib/agency";

// clientsForOwner — the owner's CRM. Client rows hold contact PII (name, email,
// phone), so the entity denies all client access; this owner-gated query is the
// only way to read them. A query has no `ctx.error`, so a non-owner gets
// `{ authorized: false }` (a bare throw would surface as a stripped
// HANDLER_ERROR) and NO data. The dashboard calls it with `callFn` and refetches
// after any write.
export default query({
  auth: "user",
  async handler(ctx): Promise<OwnerClientsResult> {
    const me = await ctx.db.get("User", ctx.auth.userId);
    const email = (me?.email as string | undefined) ?? null;
    if (!emailMatchesOwner(email, ctx.env.PYLON_OWNER_EMAIL)) {
      return { authorized: false };
    }

    const rows = (await ctx.db.unsafe.list("Client")) as unknown as ClientRow[];
    const clients = rows
      .map((r) => ({ ...r }))
      .sort((a, b) => (a.createdAt < b.createdAt ? 1 : a.createdAt > b.createdAt ? -1 : 0));

    return { authorized: true, clients };
  },
});
