import { expect, test } from "bun:test";

import type { StripeConfig } from "../types";
import { stripeWebhookHandler } from "./webhook";

// Regression for the high-severity "billing silently never syncs" bug: Stripe
// POSTs the webhook anonymously, so the handler MUST declare auth:"public".
// With the default auth:"user", every delivery to /api/fn/stripeWebhook is
// rejected with 401 AUTH_REQUIRED before the signature check runs, so no
// subscription/credit row is ever written. The signature verification inside
// the handler (STRIPE_WEBHOOK_SECRET) is the real auth boundary, not a session.
test("stripeWebhookHandler is public (Stripe POSTs anonymously)", () => {
  const cfg = {
    referenceType: "org",
    plans: [],
  } as unknown as StripeConfig;

  const handler = stripeWebhookHandler(cfg);
  expect(handler.auth).toBe("public");
});
