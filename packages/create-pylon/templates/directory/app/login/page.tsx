import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { AuthForm } from "../auth-form";
import { siteConfig } from "@/lib/site.config";
import { WRAP } from "@/components/marketing";

export const metadata: Metadata = {
  title: `Sign in — ${siteConfig.brand.name}`,
  robots: "noindex",
};

// `app/login/page.tsx` → `/login`. The owner's sign-in. It sits outside the
// `(marketing)` route group, so it renders bare — no marketing nav/footer.
// Already signed in?
// Skip straight to the dashboard — `response.redirect` in the synchronous shell
// render is a real 307 before any HTML is sent.
export default function LoginPage({ auth, response }: PageProps) {
  // Already signed in (a real account, not an anonymous guest)? Skip the form.
  if (auth.user_id && !auth.user_id.startsWith("guest_")) response.redirect("/dashboard");
  const { brand } = siteConfig;
  return (
    <div className="flex min-h-screen flex-col bg-white text-zinc-900">
      <div className="border-b border-zinc-200">
        <div className={`${WRAP} flex h-12 items-center`}>
          <Link href="/" className="text-[15px] font-semibold">
            {brand.name}
          </Link>
        </div>
      </div>
      <div className="flex flex-1 items-center justify-center px-5 py-16">
        <div className="w-full max-w-[380px]">
          <h1 className="text-[1.75rem] font-semibold tracking-[-0.02em]">Curator sign in</h1>
          <p className="mt-2 text-[14px] text-zinc-600">Review submissions and publish them to {brand.name}.</p>
          <div className="mt-8 border border-zinc-200 p-6">
            <AuthForm />
          </div>
        </div>
      </div>
    </div>
  );
}
