import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { AuthForm } from "../auth-form";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: `Sign in — ${siteConfig.brand.name}`,
  robots: "noindex",
};

// `app/login/page.tsx` → `/login`. The owner's sign-in. Rendered bare —
// /login sits outside the `(marketing)` route group, so no marketing nav or
// footer. Already signed in?
// Skip straight to the dashboard — `response.redirect` in the synchronous shell
// render is a real 307 before any HTML is sent.
export default function LoginPage({ auth, response }: PageProps) {
  // Already signed in (a real account, not an anonymous guest)? Skip the form.
  if (auth.user_id && !auth.user_id.startsWith("guest_")) response.redirect("/dashboard");
  const { brand } = siteConfig;
  return (
    <div className="flex min-h-screen flex-col bg-cream text-ink">
      <div className="border-b border-ink">
        <div className="mx-auto flex h-14 w-full max-w-5xl items-center px-6">
          <Link href="/" className="font-display text-[26px] leading-none tracking-[0.02em]">
            {brand.name}
          </Link>
        </div>
      </div>
      <div className="flex flex-1 items-center justify-center px-6 py-16">
        <div className="w-full max-w-[400px]">
          <h1 className="font-display text-[3.5rem] leading-none">Owner sign in</h1>
          <p className="mt-3 text-[15px] text-ink/70">Bookings, confirmations, and cancellations.</p>
          <div className="mt-8 border border-ink bg-paper/60 p-6">
            <AuthForm />
          </div>
        </div>
      </div>
    </div>
  );
}
