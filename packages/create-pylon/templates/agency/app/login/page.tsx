import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { AuthForm } from "../auth-form";
import { siteConfig } from "@/lib/site.config";

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
    <div className="flex min-h-screen flex-col bg-white text-ink">
      <div className="mx-auto flex h-16 w-full max-w-5xl items-center px-6">
        <Link href="/" className="font-display text-[1.5rem] leading-none">
          {brand.name}
        </Link>
      </div>
      <div className="flex flex-1 items-center justify-center px-6 pb-24">
        <div className="w-full max-w-[380px]">
          <h1 className="font-display text-[2.75rem] leading-none tracking-[-0.015em]">Studio sign in</h1>
          <p className="mt-3 text-[15px] text-zinc-600">Leads, work, clients, and invoices.</p>
          <div className="mt-8">
            <AuthForm />
          </div>
        </div>
      </div>
    </div>
  );
}
