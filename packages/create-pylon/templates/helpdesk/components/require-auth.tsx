"use client";

import React, { useState } from "react";
import { useAuth } from "@pylonsync/client";
import { BRAND } from "@/lib/brand";
import { AuthForm, type AuthMode } from "@/components/auth-form";
import { BrandMark } from "@/components/brand-mark";

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
export function RequireAuth({ children }: { children: React.ReactNode }) {
  const { isSignedIn, isLoaded } = useAuth();
  const [mode, setMode] = useState<AuthMode>("login");

  // `isLoaded` covers the beat before the engine has resolved the stored token.
  // Rendering the form during it would flash a sign-in screen at someone who is
  // already signed in.
  if (!isLoaded) {
    return (
      <main className="flex min-h-dvh items-center justify-center" aria-busy="true">
        <BrandMark className="size-8 animate-pulse" />
        <span className="sr-only">Loading {BRAND.name}</span>
      </main>
    );
  }

  if (!isSignedIn) {
    return (
      <main className="flex min-h-dvh items-center justify-center bg-surface-1 px-4 py-10">
        <div className="w-full max-w-[380px]">
          <div className="mb-6 flex items-center justify-center gap-2.5">
            <BrandMark className="size-8 rounded-[9px]" />
            <span className="text-[17px] font-semibold tracking-[-0.015em]">{BRAND.name}</span>
          </div>
          <div className="rounded-xl border border-border bg-background p-6 shadow-[0_1px_2px_rgb(0_0_0/0.04),0_12px_32px_-12px_rgb(0_0_0/0.12)] sm:p-7">
            <h1 className="text-[18px] font-semibold tracking-[-0.015em]">
              {mode === "login" ? `Sign in to ${BRAND.name}` : `Create your ${BRAND.name} account`}
            </h1>
            <p className="mt-1.5 text-[13px] leading-5 text-muted-foreground">
              {BRAND.description}
            </p>
            <div className="mt-6">
              <AuthForm mode={mode} onModeChange={setMode} />
            </div>
          </div>
        </div>
      </main>
    );
  }

  return <>{children}</>;
}
