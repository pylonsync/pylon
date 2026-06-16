import { query } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";
import type { OrderRow, OwnerOrdersResult } from "../lib/shop";

// ordersForOwner — the owner's view of every order, INCLUDING the customer's
// name + email. The one function allowed to return that PII, gated to the
// configured owner (PYLON_OWNER_EMAIL via ctx.env).
//
// The dashboard calls it with `callFn` and re-fetches whenever the live, public
// Product set changes (stock moves on every order) — so new orders show up
// without a refresh, while contact details never travel over entity sync.
export default query({
  auth: "user",
  async handler(ctx): Promise<OwnerOrdersResult> {
    const me = await ctx.db.get("User", ctx.auth.userId);
    const email = (me?.email as string | undefined) ?? null;
    if (!emailMatchesOwner(email, ctx.env.PYLON_OWNER_EMAIL)) {
      return { authorized: false };
    }

    const rows = (await ctx.db.unsafe.list("Order")) as unknown as OrderRow[];
    const orders = rows
      .map((r) => ({ ...r }))
      .sort((a, b) => (a.createdAt < b.createdAt ? 1 : a.createdAt > b.createdAt ? -1 : 0));

    return { authorized: true, orders };
  },
});
