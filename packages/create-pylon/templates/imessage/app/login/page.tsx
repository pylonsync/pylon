import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { LoginForm } from "./login-form";

export const metadata: Metadata = { title: "Sign in", robots: "noindex" };

// `/login` — the owner signs in with a one-time code sent to their email.
export default function LoginPage({ auth, response }: PageProps) {
  if (auth.user_id && !auth.user_id.startsWith("guest_") && auth.is_admin) response.redirect("/");
  return (
    <main className="flex min-h-[100dvh] items-center justify-center bg-canvas px-4">
      <div className="w-full max-w-[380px] rounded-[1.6rem] bg-black/[0.03] p-1.5 ring-1 ring-black/[0.04] dark:bg-white/[0.04]">
        <div className="rounded-[calc(1.6rem-0.375rem)] bg-background px-7 pb-7 pt-8 shadow-[0_1px_2px_rgba(0,0,0,0.04),0_24px_48px_-24px_rgba(0,0,0,0.12)]">
          <span className="flex size-11 items-center justify-center rounded-[0.9rem] bg-gradient-to-b from-[#5ac8fa] to-[#0a84ff] text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.35),0_6px_16px_-6px_rgba(10,132,255,0.7)]">
            <svg viewBox="0 0 24 24" className="size-6 fill-white" aria-hidden>
              <path d="M12 3C6.5 3 2 6.8 2 11.5c0 2.6 1.4 4.9 3.6 6.5-.2 1.3-.9 2.6-2 3.5 2.2 0 4-.8 5.2-1.8 1 .3 2.1.4 3.2.4 5.5 0 10-3.8 10-8.6S17.5 3 12 3z" />
            </svg>
          </span>
          <h1 className="mt-5 text-[22px] font-semibold tracking-[-0.02em]">Sign in to your assistant</h1>
          <p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">
            Use the owner email set in PYLON_ADMIN_EMAILS. We send a six-digit code to prove it is yours.
          </p>
          <div className="mt-6">
            <LoginForm />
          </div>
        </div>
      </div>
    </main>
  );
}
