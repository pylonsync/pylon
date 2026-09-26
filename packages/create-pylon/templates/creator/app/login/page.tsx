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
    <div className="font-display flex min-h-screen flex-col bg-paper px-6 text-ink">
      <div className="mx-auto flex h-16 w-full max-w-[40rem] items-center">
        <Link href="/" className="text-[17px] font-medium text-ink">
          {brand.name}
        </Link>
      </div>
      <div className="flex flex-1 items-center justify-center pb-24">
        <div className="w-full max-w-[380px]">
          <h1 className="text-[2rem] font-medium leading-tight">Sign in</h1>
          <p className="mt-2 text-[17px] italic text-ink-2">
            Owner access to {siteConfig.newsletter.name} subscribers.
          </p>
          <div className="mt-8 rounded-lg border border-rule bg-white p-6">
            <AuthForm />
          </div>
        </div>
      </div>
    </div>
  );
}
