import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { AuthShell } from "../auth-shell";
import { AuthForm } from "../auth-form";

export const metadata: Metadata = {
  title: "Sign in — Acme",
  robots: "noindex",
};

// `app/login/page.tsx` → `/login`. Split-screen auth (form left, brand right).
// Auth routes sit outside the `(marketing)` route group, so they render bare —
// no marketing nav or footer.
export default function LoginPage({ auth, response }: PageProps) {
  // Already signed in? Skip the form. `response.redirect` runs in the
  // synchronous shell render, so it's a real 307 before any HTML is sent.
  if (auth.user_id) response.redirect("/dashboard");
  return (
    <AuthShell
      title="Welcome back"
      switchPrompt="New to Acme?"
      switchLabel="Create an account"
      switchHref="/signup"
    >
      <AuthForm mode="login" />
    </AuthShell>
  );
}
