import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { OnboardingWizard } from "./onboarding-client";

export const metadata: Metadata = {
  title: "Set up your workspace — Acme",
  robots: "noindex",
};

// `/onboarding` — the first-run wizard: name the workspace, invite the team,
// create the first project, pick a plan. Signed-out visitors get a real 307
// to /signup. A workspace that already finished the wizard goes straight to
// the dashboard, so the URL is safe to revisit.
export default function OnboardingPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/signup");
    return null;
  }
  let org: { id: string; name: string; onboardedAt?: string | null } | null = null;
  if (auth.tenant_id) {
    org = use(
      serverData.get<{ id: string; name: string; onboardedAt?: string | null }>(
        "Org",
        auth.tenant_id,
      ),
    );
    if (org?.onboardedAt) {
      response.redirect("/dashboard");
      return null;
    }
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return <OnboardingWizard org={org} email={me?.email ?? ""} />;
}
