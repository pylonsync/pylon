import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { AuthForm } from "../auth-form";

export const metadata: Metadata = {
  title: "Sign in — Acme",
  // Auth pages shouldn't be indexed.
  robots: "noindex",
};

// `app/login/page.tsx` → `/login`. A server-rendered shell around the
// client-side <AuthForm> island, centered in the viewport.
export default function LoginPage({ auth, response }: PageProps) {
  // Already signed in? Skip the form. `response.redirect` runs in the
  // synchronous shell render, so it's a real 307 before any HTML is sent
  // (no flash, works with JS disabled).
  if (auth.user_id) response.redirect("/dashboard");
  return (
    <div className="flex min-h-screen items-center justify-center bg-white px-6 py-16">
      <div className="w-full max-w-sm">
        <Link href="/" className="mx-auto flex w-fit items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-lg bg-zinc-900 text-sm font-bold text-white">
            A
          </span>
        </Link>
        <h1 className="mt-6 text-center text-2xl font-semibold tracking-tight text-zinc-900">
          Welcome back
        </h1>
        <p className="mt-2 text-center text-sm text-zinc-500">
          Sign in to your Acme workspace.
        </p>
        <div className="mt-8 rounded-2xl border border-zinc-200 bg-white p-6 shadow-sm">
          <AuthForm mode="login" />
        </div>
        <p className="mt-6 text-center text-sm text-zinc-500">
          New to Acme?{" "}
          <Link href="/signup" className="font-medium text-zinc-900 hover:underline">
            Create an account
          </Link>
        </p>
      </div>
    </div>
  );
}
