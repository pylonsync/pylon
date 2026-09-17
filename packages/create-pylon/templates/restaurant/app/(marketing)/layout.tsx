import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";

// `(marketing)` is a ROUTE GROUP: the parens segment is stripped from every
// URL (so `(marketing)/page.tsx` still serves `/`), and this layout wraps only
// the pages inside the group — a slim nav and a footer, both driven by
// lib/site.config.ts. /login and /dashboard sit outside the group and render
// bare in the root shell. `auth.user_id` is resolved server-side from the
// session cookie before any HTML is sent, so the owner sees "Dashboard" and
// everyone else sees "Sign in" — no flash, no client fetch.
interface LayoutProps {
  children: React.ReactNode;
  auth: PageAuth;
}

export default function MarketingLayout({ children, auth }: LayoutProps) {
  // A guest session (minted by <EnsureGuest> for the live picker) has a
  // `guest_…` user id — that's an anonymous visitor, NOT the signed-in owner,
  // so it shouldn't flip the nav to "Dashboard".
  const signedIn = Boolean(auth?.user_id && !auth.user_id.startsWith("guest_"));
  const { brand } = siteConfig;

  return (
    <>
      {/* The nav sits over the hero photo: dark, translucent, cream type. */}
      <header className="sticky top-0 z-30 border-b border-white/10 bg-[var(--ink)]/80 text-[var(--cream)] backdrop-blur">
        <div className="mx-auto flex h-16 max-w-6xl items-center justify-between px-6">
          <Link href="/" className="font-display text-[22px] tracking-tight">
            {brand.name}
          </Link>
          <nav className="flex items-center gap-1 sm:gap-2">
            <a href="/#menu" className="hidden px-3 py-1.5 text-[13px] font-medium text-[var(--cream-2)] transition-colors hover:text-[var(--cream)] sm:inline-flex">
              Menu
            </a>
            <a href="/#visit" className="hidden px-3 py-1.5 text-[13px] font-medium text-[var(--cream-2)] transition-colors hover:text-[var(--cream)] sm:inline-flex">
              Visit
            </a>
            {signedIn ? (
              <Link href="/dashboard" className="inline-flex items-center rounded-full border border-white/20 px-3.5 py-1.5 text-[13px] font-medium transition-colors hover:bg-white/10">
                Dashboard
              </Link>
            ) : (
              <>
                <Link href="/login" className="hidden px-3 py-1.5 text-[13px] font-medium text-[var(--cream-2)] transition-colors hover:text-[var(--cream)] sm:inline-flex">
                  Sign in
                </Link>
                <a href="/#reserve" className="inline-flex items-center rounded-full bg-brand px-4 py-2 text-[13px] font-medium text-white transition-opacity hover:opacity-90">
                  {siteConfig.hero.ctaLabel}
                </a>
              </>
            )}
          </nav>
        </div>
      </header>

      <main className="flex-1">{children}</main>

      <SiteFooter />
    </>
  );
}

function SiteFooter() {
  const { brand, location } = siteConfig;
  return (
    <footer className="border-t border-white/10 bg-[var(--ink)] text-[var(--cream-2)]">
      <div className="mx-auto max-w-6xl px-6 py-14">
        <div className="flex flex-col items-start justify-between gap-8 sm:flex-row">
          <div className="max-w-sm">
            <Link href="/" className="font-display text-[26px] tracking-tight text-[var(--cream)]">
              {brand.name}
            </Link>
            <p className="mt-3 text-[13.5px] leading-relaxed">{brand.footerBlurb}</p>
          </div>
          <div className="text-[13.5px] leading-relaxed">
            <div className="font-medium text-[var(--cream)]">Visit</div>
            <p className="mt-2 max-w-[14rem]">{location.address}</p>
            <p className="mt-2">{location.phone}</p>
            <a href={`mailto:${brand.email}`} className="mt-1 inline-block transition-colors hover:text-[var(--cream)]">
              {brand.email}
            </a>
          </div>
        </div>
        <div className="mt-12 flex flex-col items-start justify-between gap-3 border-t border-white/10 pt-6 text-[12px] sm:flex-row sm:items-center">
          <span>© {new Date().getFullYear()} {brand.copyrightName}</span>
          <span>
            Built with{" "}
            <a href="https://pylonsync.com" className="font-medium text-[var(--cream)]">
              Pylon
            </a>
          </span>
        </div>
      </div>
    </footer>
  );
}
