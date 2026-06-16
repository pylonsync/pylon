import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { AuthForm } from "../auth-form";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: `Sign in — ${siteConfig.brand.name}`,
  robots: "noindex",
};

// `app/login/page.tsx` → `/login`. The owner's sign-in. Rendered bare (the
// layout suppresses the marketing nav/footer for /login). Already signed in?
// Skip straight to the dashboard — `response.redirect` in the synchronous shell
// render is a real 307 before any HTML is sent.
export default function LoginPage({ auth, response }: PageProps) {
  // Already signed in (a real account, not an anonymous guest)? Skip the form.
  if (auth.user_id && !auth.user_id.startsWith("guest_")) response.redirect("/dashboard");
  const { brand } = siteConfig;
  return (
    <div className="flex min-h-screen items-center justify-center bg-white px-6 py-12">
      <div className="w-full max-w-[400px] rounded-2xl border border-zinc-200/70 p-8">
        <Link href="/" className="inline-flex">
          <span className="flex size-9 items-center justify-center rounded-xl bg-zinc-900 text-base font-bold text-white">
            {brand.letter}
          </span>
        </Link>
        <h1 className="mt-5 text-[22px] font-semibold tracking-tight text-zinc-900">
          {brand.name} dashboard
        </h1>
        <p className="mt-1 text-[13px] text-zinc-500">
          Sign in to see your subscribers.
        </p>
        <div className="mt-6">
          <AuthForm />
        </div>
      </div>
    </div>
  );
}
