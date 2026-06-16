import { action } from "@pylonsync/functions";
import { verifyStripeSignature } from "@pylonsync/stripe";

// stripeWebhook — Stripe's callback that settles a checkout. Mounted at
// POST /api/webhooks/stripeWebhook (the webhook route gives an action the EXACT
// raw request bytes Stripe signed; `/api/fn/...` would re-encode them and break
// the signature). Point your Stripe dashboard webhook there and put the signing
// secret in STRIPE_WEBHOOK_SECRET.
//
// SECURITY: the request is unauthenticated, so the HMAC signature IS the auth.
// We verify it with the vetted constant-time `verifyStripeSignature` BEFORE
// trusting a single byte — never parse-then-trust. A bad/missing/stale/replayed
// signature is rejected with 400.
//
// On success it settles the cart's order lines by `client_reference_id` (the
// orderGroupId set at checkout): paid → mark "paid"; expired/failed → release
// the held stock. Both settle mutations are idempotent, so Stripe's retries are
// safe.
export default action({
  auth: "public",
  args: {},
  async handler(ctx) {
    if (!ctx.request) {
      throw ctx.error("BAD_INVOCATION", "stripeWebhook must be called as a webhook.");
    }
    const secret = ctx.env.STRIPE_WEBHOOK_SECRET?.trim();
    if (!secret) {
      throw ctx.error("NOT_CONFIGURED", "STRIPE_WEBHOOK_SECRET is not set.");
    }

    const sig = ctx.request.headers["stripe-signature"];
    const verdict = await verifyStripeSignature(secret, ctx.request.rawBody, sig);
    if (verdict !== true) {
      throw ctx.error("INVALID_SIGNATURE", `stripe signature: ${verdict}`);
    }

    let event: {
      type?: string;
      data?: { object?: { client_reference_id?: string; payment_status?: string; metadata?: { orderGroupId?: string } } };
    };
    try {
      event = JSON.parse(ctx.request.rawBody);
    } catch {
      throw ctx.error("BAD_BODY", "Webhook body is not valid JSON.");
    }

    const session = event.data?.object ?? {};
    const orderGroupId = session.client_reference_id || session.metadata?.orderGroupId;

    switch (event.type) {
      case "checkout.session.completed":
        // Card payments are paid on completion; delayed methods arrive as
        // "unpaid" here and confirm later via async_payment_succeeded.
        if (orderGroupId && session.payment_status === "paid") {
          await ctx.runMutation("markGroupPaid", { orderGroupId });
        }
        return { ok: true, type: event.type };

      case "checkout.session.async_payment_succeeded":
        if (orderGroupId) await ctx.runMutation("markGroupPaid", { orderGroupId });
        return { ok: true, type: event.type };

      case "checkout.session.expired":
      case "checkout.session.async_payment_failed":
        if (orderGroupId) await ctx.runMutation("releaseGroup", { orderGroupId });
        return { ok: true, type: event.type };

      default:
        return { ignored: event.type ?? "unknown" };
    }
  },
});
