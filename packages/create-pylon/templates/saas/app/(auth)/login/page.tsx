import React from "react";
import { Link, type Metadata } from "@pylonsync/react";
import { AuthForm } from "../auth-form";

export const metadata: Metadata = {
  title: "Sign in — Acme",
  robots: "noindex",
};

// `(auth)/login/page.tsx` → `/login`. The `(auth)` layout owns the
// split-screen frame and the already-signed-in redirect; this page is just
// the heading, the switch link, and the form.
export default function LoginPage() {
  return (
    <>
      <h1 className="mt-5 text-[22px] font-semibold tracking-tight text-zinc-900">
        Welcome back
      </h1>
      <p className="mt-1 text-[13px] text-zinc-500">
        New to Acme?{" "}
        <Link
          href="/signup"
          className="font-medium text-zinc-900 underline underline-offset-2"
        >
          Create an account
        </Link>
      </p>
      <div className="mt-6">
        <AuthForm mode="login" />
      </div>
    </>
  );
}
