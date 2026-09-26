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
    <div className="flex min-h-screen flex-col bg-ink px-6 text-chalk">
      <div className="mx-auto flex h-16 w-full max-w-5xl items-center">
        <Link href="/" className="font-display text-[17px] font-bold tracking-tight text-chalk">
          {brand.name}
        </Link>
      </div>
      <div className="flex flex-1 items-center justify-center pb-24">
        <div className="w-full max-w-[380px]">
          <h1 className="font-display text-[1.75rem] font-bold leading-tight tracking-[-0.02em]">
            Sign in to the waitlist
          </h1>
          <p className="mt-2 text-[14px] text-chalk-2">Owner access to signups and the export.</p>
          <div className="mt-8 rounded-lg border border-line bg-paper p-6">
            <AuthForm />
          </div>
        </div>
      </div>
    </div>
  );
}
