import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import { Billing, type Subscription } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Billing — Acme",
  robots: "noindex",
};

// `/dashboard/billing` — the active workspace's plan + Stripe checkout/portal.
// The StripeSubscription row is resolved server-side (the @pylonsync/stripe
// read policy scopes it to the active tenant), so the plan paints with no flash;
// upgrade/manage open Stripe and the webhook keeps the row in sync.
const ACTIVE = ["active", "trialing", "past_due"];

export default function BillingPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  const org = auth.tenant_id
    ? use(serverData.get<{ name?: string }>("Org", auth.tenant_id))
    : null;
  const subs = auth.tenant_id
    ? use(serverData.list<Subscription>("StripeSubscription"))
    : [];
  const subscription =
    subs.find(
      (s) => s.referenceId === auth.tenant_id && ACTIVE.includes(s.status),
    ) ??
    subs[0] ??
    null;
  return (
    <DashboardShell
      active="billing"
      title="Billing"
      userEmail={me?.email ?? ""}
      orgName={org?.name}
    >
      <Billing
        tenantId={auth.tenant_id}
        role={auth.roles?.[0] ?? ""}
        subscription={subscription}
      />
    </DashboardShell>
  );
}
