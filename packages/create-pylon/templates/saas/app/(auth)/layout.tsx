import React from "react";
import { Link, type PageAuth, type SsrResponse } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";

// `(auth)` route group → the shared split-screen frame for /login and /signup:
// the form on the left, a product screenshot on the right (hidden on small
// screens). Each page supplies what differs — heading, switch link, and
// the form — as `children`. The group segment adds no URL prefix, and because
// these routes sit outside `(marketing)`, none of the site nav/footer renders
// here.
interface LayoutProps {
  children: React.ReactNode;
  auth: PageAuth;
  response: SsrResponse;
}

export default function AuthLayout({ children, auth, response }: LayoutProps) {
  // Already signed in? Skip the auth screens entirely. Gating here covers
  // every page in the group; `response.redirect` runs in the synchronous
  // shell render, so it's a real 307 before any HTML is sent.
  if (auth.user_id) {
    response.redirect("/dashboard");
    return null;
  }
  return (
    <div className="grid min-h-screen bg-white lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
      {/* Form side */}
      <div className="flex items-center justify-center px-4 py-12 sm:px-6">
        <div className="w-full max-w-[380px]">
          <Link href="/" className="inline-flex items-center gap-2">
            <span className="flex size-8 items-center justify-center rounded-[9px] bg-zinc-950 text-[15px] font-bold text-white">
              {siteConfig.brand.letter}
            </span>
            <span className="text-[16px] font-semibold tracking-tight text-zinc-950">{siteConfig.brand.name}</span>
          </Link>
          {children}
        </div>
      </div>

      {/* Product side: a capture of this app's own task board, cropped so it
          runs off the right edge. */}
      <div className="relative hidden overflow-hidden bg-zinc-50 lg:block">
        <div
          aria-hidden
          className="absolute inset-0 bg-[radial-gradient(70%_60%_at_30%_20%,var(--brand-soft),transparent_70%)]"
        />
        <div className="relative flex h-full flex-col justify-center py-16 pl-14">
          <p className="max-w-md text-[1.5rem] font-semibold leading-snug tracking-[-0.02em] text-zinc-950">
            {siteConfig.hero.headline}
          </p>
          <div className="mt-10 w-[900px] rounded-l-[20px] bg-zinc-950/[0.035] py-2 pl-2 ring-1 ring-zinc-950/[0.06]">
            <img
              src="/screenshots/board.png"
              alt="A project board in Acme"
              className="block w-full rounded-l-[14px] shadow-[0_0_0_1px_rgba(0,0,0,0.06),0_24px_48px_-24px_rgba(24,24,60,0.25)]"
            />
          </div>
        </div>
      </div>
    </div>
  );
}
