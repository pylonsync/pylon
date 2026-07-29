import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { AuthShell } from "../auth-shell";
import { AuthForm } from "../auth-form";

export const metadata: Metadata = {
  title: "Create your account — Acme",
  robots: "noindex",
};

// `app/signup/page.tsx` → `/signup`. Split-screen auth, register mode.
export default function SignupPage({ auth, response }: PageProps) {
  if (auth.user_id) response.redirect("/dashboard");
  return (
    <AuthShell
      title="Create an account"
      switchPrompt="Already have an account?"
      switchLabel="Sign in"
      switchHref="/login"
    >
      <AuthForm mode="signup" />
    </AuthShell>
  );
}
