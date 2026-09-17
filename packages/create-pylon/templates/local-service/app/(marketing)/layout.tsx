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

const NAV_LINK =
  "hidden px-3 py-1.5 text-[13px] font-medium text-ink/70 transition-colors hover:text-ink sm:inline-flex";

export default function MarketingLayout({ children, auth }: LayoutProps) {
  // A guest session (minted by <EnsureGuest> for the live picker) has a
  // `guest_…` user id — that's an anonymous visitor, NOT the signed-in owner,
  // so it shouldn't flip the nav to "Dashboard".
  const signedIn = Boolean(auth?.user_id && !auth.user_id.startsWith("guest_"));
  const { brand } = siteConfig;

  return (
    <>
      <header className="sticky top-0 z-30 border-b border-ink bg-cream text-ink">
        <div className="mx-auto flex h-14 max-w-5xl items-center justify-between px-6">
          <Link href="/" className="font-display text-[26px] leading-none tracking-[0.02em]">
            {brand.name}
          </Link>
          <nav className="flex items-center gap-1 sm:gap-2">
            <a href="/#services" className={NAV_LINK}>
              Prices
            </a>
            <a href="/#visit" className={NAV_LINK}>
              Visit
            </a>
            {signedIn ? (
              <Link
                href="/dashboard"
                className="inline-flex h-9 items-center bg-ink px-4 text-[13px] font-medium text-cream transition-opacity hover:opacity-90"
              >
                Dashboard
              </Link>
            ) : (
              <>
                <Link href="/login" className={NAV_LINK}>
                  Sign in
                </Link>
                <a
                  href="/#book"
                  className="inline-flex h-9 items-center bg-brand px-4 text-[13px] font-medium text-cream transition-opacity hover:opacity-90"
                >
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
    <footer className="border-t border-ink bg-cream text-ink">
      <div className="mx-auto max-w-5xl px-6 py-12">
        <div className="flex flex-col items-start justify-between gap-8 sm:flex-row">
          <div className="max-w-sm">
            <Link href="/" className="font-display text-[32px] leading-none tracking-[0.02em]">
              {brand.name}
            </Link>
            <p className="mt-3 text-[13.5px] leading-relaxed text-ink/70">{brand.footerBlurb}</p>
          </div>
          <div className="text-[13.5px] leading-relaxed text-ink/70">
            <div className="font-medium text-ink">Visit</div>
            <p className="mt-2 max-w-[14rem]">{location.address}</p>
            <p className="mt-2">{location.phone}</p>
            <a href={`mailto:${brand.email}`} className="mt-1 inline-block transition-colors hover:text-ink">
              {brand.email}
            </a>
          </div>
        </div>
        <div className="mt-10 flex flex-col items-start justify-between gap-3 border-t border-ink pt-6 text-[12px] text-ink/60 sm:flex-row sm:items-center">
          <span>
            © {new Date().getFullYear()} {brand.copyrightName}
          </span>
          <span>
            Built with{" "}
            <a href="https://pylonsync.com" className="font-medium text-ink/80 hover:text-ink">
              Pylon
            </a>
          </span>
        </div>
      </div>
    </footer>
  );
}
