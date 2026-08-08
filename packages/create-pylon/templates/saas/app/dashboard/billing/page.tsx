import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { Billing, type Subscription } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Billing — Acme",
  robots: "noindex",
};

// `/dashboard/billing` — the active workspace's plan + Stripe checkout/portal.
// The StripeSubscription row is resolved server-side (the @pylonsync/stripe
// read policy scopes it to the active tenant), so the plan paints with no flash;
// upgrade/manage open Stripe and the webhook keeps the row in sync. Auth gate +
// shell chrome come from the dashboard layout.
const ACTIVE = ["active", "trialing", "past_due"];

export default function BillingPage({ auth, response, serverData }: PageProps) {
  if (!auth.tenant_id) {
    response.redirect("/dashboard");
    return null;
  }
  const subs = use(serverData.list<Subscription>("StripeSubscription"));
  const subscription =
    subs.find(
      (s) => s.referenceId === auth.tenant_id && ACTIVE.includes(s.status),
    ) ??
    subs[0] ??
    null;
  return (
    <Billing
      tenantId={auth.tenant_id}
      role={auth.roles?.[0] ?? ""}
      subscription={subscription}
    />
  );
}
