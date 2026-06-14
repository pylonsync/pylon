import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { AuthForm } from "../auth-form";

export const metadata: Metadata = {
  title: "Create your account — Acme",
  robots: "noindex",
};

// `app/signup/page.tsx` → `/signup`. Same centered shell as /login, register
// mode.
export default function SignupPage({ auth, response }: PageProps) {
  if (auth.user_id) response.redirect("/dashboard");
  return (
    <div className="flex min-h-[calc(100vh-3.5rem)] items-center justify-center bg-white px-6 py-16">
      <div className="w-full max-w-sm">
        <Link href="/" className="mx-auto flex w-fit items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-lg bg-zinc-900 text-sm font-bold text-white">
            A
          </span>
        </Link>
        <h1 className="mt-6 text-center text-2xl font-semibold tracking-tight text-zinc-900">
          Create your account
        </h1>
        <p className="mt-2 text-center text-sm text-zinc-500">
          Start your Acme workspace — free, no credit card.
        </p>
        <div className="mt-8 rounded-2xl border border-zinc-200 bg-white p-6 shadow-sm">
          <AuthForm mode="signup" />
        </div>
        <p className="mt-6 text-center text-sm text-zinc-500">
          Already have an account?{" "}
          <Link href="/login" className="font-medium text-zinc-900 hover:underline">
            Sign in
          </Link>
        </p>
      </div>
    </div>
  );
}
