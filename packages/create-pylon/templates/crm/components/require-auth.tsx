"use client";

import React from "react";
import { useAuth } from "@pylonsync/client";
import { AuthForm } from "@/components/auth-form";

/**
 * Client-side auth gate.
 *
 * Deliberately NOT a server-side `response.redirect("/login")`. The session
 * cookie is `SameSite=Lax`, which browsers withhold on cross-site iframe
 * navigations — and this app is shown inside an iframe in the Pylon Cloud
 * builder's live preview. An SSR gate there sees every request as anonymous no
 * matter how many times you sign in, so signing in bounces straight back to the
 * form. The token in local storage is visible to the client either way, so the
 * gate belongs here.
 *
 * It also removes a full page load from the sign-in path: authenticating swaps
 * this component's children in place instead of navigating.
 */
export function RequireAuth({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  const { isSignedIn, isLoaded } = useAuth();

  // `isLoaded` covers the beat before the engine has resolved the stored token.
  // Rendering the form during it would flash a sign-in screen at someone who is
  // already signed in.
  if (!isLoaded) {
    return (
      <main className="flex min-h-screen items-center justify-center">
        <span className="text-[13px] text-muted-foreground">Loading…</span>
      </main>
    );
  }

  if (!isSignedIn) {
    return (
      <main className="flex min-h-screen items-center justify-center px-6">
        <div className="w-full max-w-[320px]">
          <div className="mb-7 flex items-center gap-2">
            <span className="flex size-7 items-center justify-center rounded-md bg-primary text-[12px] font-bold text-primary-foreground">
              {title.slice(0, 1).toUpperCase()}
            </span>
            <span className="text-[15px] font-semibold tracking-tight">{title}</span>
          </div>
          <h1 className="text-[19px] font-semibold tracking-tight">
            Sign in to your workspace
          </h1>
          <p className="mt-1 text-[13px] text-muted-foreground">{description}</p>
          <div className="mt-6">
            <AuthForm />
          </div>
        </div>
      </main>
    );
  }

  return <>{children}</>;
}
