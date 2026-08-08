import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";

// `(marketing)` is a ROUTE GROUP: the parens segment is stripped from every
// URL (so `(marketing)/page.tsx` still serves `/`), and this layout wraps
// only the pages inside the group — a slim nav up top and a footer below,
// both driven by lib/site.config.ts. Routes outside the group (/login,
// /dashboard) render bare in the root shell. `auth.user_id` is resolved
// server-side from the session cookie before any HTML is sent, so the nav
// shows "Dashboard" once the owner is signed in and "Sign in" otherwise —
// no flash, no client fetch.
interface LayoutProps {
  children: React.ReactNode;
  auth: PageAuth;
}

export default function MarketingLayout({ children, auth }: LayoutProps) {
  // A guest session (minted by <EnsureGuest> for the live counter) has a
  // `guest_…` user id — that's an anonymous visitor, NOT the signed-in owner,
  // so it shouldn't flip the nav to "Dashboard".
  const signedIn = Boolean(auth?.user_id && !auth.user_id.startsWith("guest_"));
  const { brand } = siteConfig;

  return (
    <>
      <header className="sticky top-0 z-30 border-b border-zinc-200/70 bg-white/85 backdrop-blur">
        <div className="mx-auto flex h-14 max-w-5xl items-center justify-between px-6">
          <Link href="/" className="flex items-center gap-2">
            <span className="flex size-6 items-center justify-center rounded-[7px] bg-zinc-900 text-[13px] font-bold text-white">
              {brand.letter}
            </span>
            <span className="text-[15px] font-semibold tracking-tight text-zinc-900">
              {brand.name}
            </span>
          </Link>
          <nav className="flex items-center gap-1 sm:gap-2">
            <a href="/#browse" className="hidden rounded-full px-3 py-1.5 text-[13px] font-medium text-zinc-600 transition-colors hover:text-zinc-900 sm:inline-flex">
              Browse
            </a>
            {signedIn ? (
              <Link
                href="/dashboard"
                className="inline-flex items-center rounded-full bg-zinc-900 px-3.5 py-1.5 text-[13px] font-medium text-white transition-colors hover:bg-zinc-700"
              >
                Dashboard
              </Link>
            ) : (
              <>
                <Link
                  href="/login"
                  className="hidden rounded-full px-3 py-1.5 text-[13px] font-medium text-zinc-600 transition-colors hover:text-zinc-900 sm:inline-flex"
                >
                  Sign in
                </Link>
                <Link
                  href="/submit"
                  className="inline-flex items-center rounded-full bg-brand px-3.5 py-1.5 text-[13px] font-medium text-white transition-opacity hover:opacity-90"
                >
                  {siteConfig.hero.ctaLabel}
                </Link>
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
  const { brand } = siteConfig;
  return (
    <footer className="border-t border-zinc-200/70 bg-white">
      <div className="mx-auto max-w-5xl px-6 py-12">
        <div className="flex flex-col items-start justify-between gap-6 sm:flex-row">
          <div className="max-w-sm">
            <Link href="/" className="inline-flex items-center gap-2">
              <span className="flex size-6 items-center justify-center rounded-[7px] bg-zinc-900 text-[13px] font-bold text-white">
                {brand.letter}
              </span>
              <span className="text-[15px] font-semibold tracking-tight text-zinc-900">
                {brand.name}
              </span>
            </Link>
            <p className="mt-3 text-[13px] leading-relaxed text-zinc-500">
              {brand.footerBlurb}
            </p>
          </div>
          <div className="flex items-center gap-4">
            {brand.socials.map((s) => (
              <a
                key={s.label}
                href={s.href}
                aria-label={s.label}
                className="text-zinc-400 transition-colors hover:text-zinc-900"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
                  <path d={s.path} />
                </svg>
              </a>
            ))}
            <a
              href={`mailto:${brand.email}`}
              aria-label="Email"
              className="text-zinc-400 transition-colors hover:text-zinc-900"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <rect x="3" y="5" width="18" height="14" rx="2" />
                <path d="m3 7 9 6 9-6" />
              </svg>
            </a>
          </div>
        </div>
        <div className="mt-10 flex flex-col items-start justify-between gap-3 border-t border-zinc-200/70 pt-6 text-[12px] text-zinc-400 sm:flex-row sm:items-center">
          <span>
            © {new Date().getFullYear()} {brand.copyrightName}
          </span>
          <span>
            Built with{" "}
            <a href="https://pylonsync.com" className="font-medium text-zinc-600 hover:text-zinc-900">
              Pylon
            </a>
          </span>
        </div>
      </div>
    </footer>
  );
}
