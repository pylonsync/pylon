import { action, v } from "@pylonsync/functions";
import { stripeRequest, assertSafeRedirectUrl } from "@pylonsync/stripe";
import type { CheckoutResult } from "../lib/shop";

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const CURRENCY = "usd";
// Stripe requires a Checkout Session to expire ≥30 min out; we hold the cart's
// stock until then, and the `checkout.session.expired` webhook returns it.
const SESSION_TTL_SECS = 35 * 60;

// checkout — turn a cart into a real order. An `action` (not a mutation) because
// it does network I/O (the Stripe API) on top of the DB write:
//
//   1. validate the cart + customer,
//   2. `runMutation("reserveCart")` to HOLD stock and record the order lines
//      (race-safe, transactional — see reserveCart.ts), and
//   3. if STRIPE_SECRET_KEY is set, open a Stripe Checkout Session and hand the
//      browser its URL; otherwise leave the order "reserved" for the owner.
//
// `auth: "public"` — a shopper has no account. PRIVACY: it returns only a redirect
// URL / status, never an Order row or anyone's email. The price is taken from the
// catalog inside reserveCart, never trusted from the client.
export default action<
  {
    items: { slug: string; qty: number }[];
    customerName: string;
    customerEmail: string;
    successUrl: string;
    cancelUrl: string;
  },
  CheckoutResult
>({
  auth: "public",
  args: {
    items: v.array(v.object({ slug: v.string(), qty: v.int() })),
    customerName: v.string(),
    customerEmail: v.string(),
    successUrl: v.string(),
    cancelUrl: v.string(),
  },
  async handler(ctx, args): Promise<CheckoutResult> {
    const name = args.customerName.trim();
    const email = args.customerEmail.trim().toLowerCase();
    if (name.length < 1 || name.length > 120) {
      throw ctx.error("INVALID_ARGS", "Enter your name.");
    }
    if (!EMAIL_RE.test(email) || email.length > 254) {
      throw ctx.error("INVALID_ARGS", "Enter a valid email address.");
    }
    const items = args.items.filter((i) => i && i.slug && Math.trunc(i.qty) >= 1);
    if (items.length === 0) return { ok: false, reason: "empty", soldOut: [] };

    const secretKey = ctx.env.STRIPE_SECRET_KEY?.trim();
    const stripeOn = Boolean(secretKey);

    // One id ties the cart's order lines together AND becomes the Stripe
    // session's client_reference_id, so the webhook settles them as a unit.
    const orderGroupId = crypto.randomUUID();

    const held = await ctx.runMutation<{
      ok: boolean;
      lines: { slug: string; name: string; qty: number; unitPriceCents: number }[];
      soldOut: string[];
    }>("reserveCart", {
      orderGroupId,
      initialStatus: stripeOn ? "pending" : "reserved",
      customerName: name,
      customerEmail: email,
      items,
    });

    if (!held.ok || held.lines.length === 0) {
      return { ok: false, reason: "sold_out", soldOut: held.soldOut };
    }

    // No payment processor → the order is held as "reserved"; the owner follows
    // up with a payment link. The store still works end-to-end with zero config.
    if (!stripeOn) {
      return { ok: true, mode: "reserved", orderGroupId, soldOut: held.soldOut };
    }

    // Stripe is configured → open a hosted Checkout Session. `price_data` prices
    // each line inline, so there are no Stripe Price objects to pre-create.
    // assertSafeRedirectUrl blocks an attacker-supplied success/cancel URL from
    // pointing the post-payment redirect off to a malicious host: it allows the
    // app's own host (PYLON_PUBLIC_URL / SITE_URL, plus localhost in dev).
    const publicUrl = ctx.env.PYLON_PUBLIC_URL || ctx.env.SITE_URL;
    const urlOpts = { extraOrigins: ctx.env.PYLON_CORS_ORIGIN };
    assertSafeRedirectUrl(args.successUrl, publicUrl, urlOpts);
    assertSafeRedirectUrl(args.cancelUrl, publicUrl, urlOpts);

    // If the session can't be created (bad key, network error, Stripe down) we
    // MUST release the held stock — there's no session that would ever expire to
    // do it for us, so without this the units would be stranded off the shelf.
    let session: { id: string; url: string } | undefined;
    try {
      session = await stripeRequest<{ id: string; url: string }>(
        { secretKey: secretKey as string, idempotencyKey: orderGroupId },
        "POST",
        "/checkout/sessions",
        {
          mode: "payment",
          success_url: args.successUrl,
          cancel_url: args.cancelUrl,
          client_reference_id: orderGroupId,
          customer_email: email,
          expires_at: Math.floor(Date.now() / 1000) + SESSION_TTL_SECS,
          metadata: { orderGroupId },
          line_items: held.lines.map((l) => ({
            quantity: l.qty,
            price_data: {
              currency: CURRENCY,
              unit_amount: l.unitPriceCents,
              product_data: { name: l.name },
            },
          })),
        },
      );
    } catch {
      session = undefined;
    }

    if (!session?.url) {
      await ctx.runMutation("releaseGroup", { orderGroupId });
      throw ctx.error("STRIPE_ERROR", "Could not start checkout. Please try again.");
    }

    return { ok: true, mode: "stripe", checkoutUrl: session.url, soldOut: held.soldOut };
  },
});
