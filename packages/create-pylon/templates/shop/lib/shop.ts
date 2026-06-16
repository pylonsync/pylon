// Shared shop types. The Order row is what the owner dashboard sees (with PII);
// the client imports only the type, never server code.

export interface OrderRow {
  id: string;
  orderGroupId: string;
  productSlug: string;
  productName: string;
  qty: number;
  unitPriceCents: number;
  customerName: string;
  customerEmail: string;
  // "pending" | "paid" | "reserved" | "fulfilled" | "cancelled"
  status: string;
  createdAt: string;
}

// ordersForOwner returns a discriminated result rather than throwing on a
// non-owner (a query has no `ctx.error`; a bare throw becomes a stripped
// HANDLER_ERROR). A non-owner gets `{ authorized: false }` and NO data.
export type OwnerOrdersResult =
  | { authorized: true; orders: OrderRow[] }
  | { authorized: false };

// One cart line on its way to checkout. The client sends slug + qty; the
// server re-prices from the catalog so the price is never client-trusted.
export interface CheckoutItem {
  slug: string;
  qty: number;
}

// What `checkout` returns to the client.
//   • mode "stripe"   → redirect the browser to `checkoutUrl` (hosted Stripe).
//   • mode "reserved" → no payment processor configured; the order is held and
//                       the owner follows up. Show the confirmation message.
// `soldOut` lists any cart lines that sold out between page-load and checkout
// (they were dropped from the order); the rest still went through.
export type CheckoutResult =
  | { ok: true; mode: "stripe"; checkoutUrl: string; soldOut: string[] }
  | { ok: true; mode: "reserved"; orderGroupId: string; soldOut: string[] }
  | { ok: false; reason: "empty" | "sold_out" | "invalid"; soldOut: string[] };

export function formatPrice(cents: number): string {
  return `$${(cents / 100).toFixed(cents % 100 === 0 ? 0 : 2)}`;
}
