import { stripe } from "@pylonsync/stripe";
import { TRIAL_DAYS } from "./plans";

// Per-workspace (org) billing. The active org IS the billing reference: its
// subscription decides the plan, and only owners/admins can change it (the
// plugin enforces that for `referenceType: "org"`). The Stripe customer is
// created on first checkout and stored on `Org.stripeCustomerId`.
//
// Configure with env vars (nothing is hard-coded so test/live swap cleanly):
//   STRIPE_SECRET_KEY       sk_test_… / sk_live_…   (required to charge)
//   STRIPE_WEBHOOK_SECRET   whsec_…                 (required for the webhook)
//   STRIPE_PRICE_PRO        price_…                 (the "pro" monthly price)
//   STRIPE_PRICE_PRO_ANNUAL price_…                 (the "pro" yearly price)
// Until STRIPE_SECRET_KEY is set the handlers return STRIPE_NOT_CONFIGURED and
// the Billing page shows a "connect Stripe" state — the app still boots + runs.
//
// Prices and the trial length are shown to customers from lib/plans.ts; keep
// the Stripe prices and that catalog in agreement.
export const billing = stripe({
  referenceType: "org",
  plans: [
    {
      name: "pro",
      priceId: process.env.STRIPE_PRICE_PRO ?? "",
      annualPriceId: process.env.STRIPE_PRICE_PRO_ANNUAL || undefined,
      // Card up front, no charge until the trial ends. Stripe allows one
      // trial per customer; the plugin refuses a second.
      freeTrial: { days: TRIAL_DAYS },
      limits: { projects: -1, seats: -1 },
    },
  ],
});

// Pylon's file-based function loader needs one default export per function
// file, so each handler gets a one-line wrapper under functions/. Re-export the
// public actions + the plugin-internal `_pylonStripe*` handlers (called via
// ctx.runQuery / ctx.runMutation) so those wrappers have something to import.
const h = billing.handlers as Record<string, unknown>;

export const createCheckoutSession = h.createCheckoutSession;
export const createBillingPortalSession = h.createBillingPortalSession;
export const cancelSubscription = h.cancelSubscription;
export const restoreSubscription = h.restoreSubscription;
export const stripeWebhook = h.stripeWebhook;

export const _pylonStripeListSubsForReference = h._pylonStripeListSubsForReference;
export const _pylonStripeFindActiveSubForReference =
  h._pylonStripeFindActiveSubForReference;
export const _pylonStripeGetCustomerHolder = h._pylonStripeGetCustomerHolder;
export const _pylonStripeFindByCustomerId = h._pylonStripeFindByCustomerId;
export const _pylonStripeOrgMembership = h._pylonStripeOrgMembership;
export const _pylonStripeSetCustomerId = h._pylonStripeSetCustomerId;
export const _pylonStripeUpsertSubscription = h._pylonStripeUpsertSubscription;
