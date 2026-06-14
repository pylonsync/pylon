import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { Onboarding } from "./onboarding-client";

export const metadata: Metadata = {
  title: "Get started — Acme",
  robots: "noindex",
};

// `/onboarding` — first-run flow. Server-side gate: signed-out users go to
// /login; users who already have an active workspace (auth.tenant_id) skip
// straight to the dashboard, so this is only ever the brand-new-account screen.
// The signed-in email is resolved server-side (serverData.get) and passed in.
export default function OnboardingPage({
  auth,
  response,
  serverData,
}: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  if (auth.tenant_id) {
    response.redirect("/dashboard");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return <Onboarding email={me?.email ?? ""} />;
}
