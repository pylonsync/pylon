import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";
import { WRAP } from "@/components/marketing";

// `(marketing)` is a ROUTE GROUP: the parens segment is stripped from every
// URL (so `(marketing)/page.tsx` still serves `/`), and this layout wraps
// only the pages inside the group — a plain text nav up top and a footer
// below, both driven by lib/site.config.ts. Routes outside the group (/login,
// /dashboard) render bare in the root shell. `auth.user_id` is resolved
// server-side from the session cookie before any HTML is sent, so the nav
// shows "Dashboard" once the owner is signed in and "Sign in" otherwise —
// no flash, no client fetch.
interface LayoutProps {
  children: React.ReactNode;
  auth: PageAuth;
}

const NAV_LINK = "text-[14px] text-zinc-700 transition-colors hover:text-ink";

export default function MarketingLayout({ children, auth }: LayoutProps) {
  // A guest session (minted by <EnsureGuest> for the live counter) has a
  // `guest_…` user id — that's an anonymous visitor, NOT the signed-in owner,
  // so it shouldn't flip the nav to "Dashboard".
  const signedIn = Boolean(auth?.user_id && !auth.user_id.startsWith("guest_"));
  const { brand } = siteConfig;

  return (
    <>
      <header className="bg-white">
        <div className={`${WRAP} flex h-16 items-center justify-between`}>
          <Link href="/" className="font-display text-[1.5rem] leading-none text-ink">
            {brand.name}
          </Link>
          <nav className="flex items-center gap-5 sm:gap-7">
            <a href="/work" className={NAV_LINK}>
              Work
            </a>
            <a href="/#services" className={`hidden sm:inline ${NAV_LINK}`}>
              Services
            </a>
            {signedIn ? (
              <Link href="/dashboard" className={NAV_LINK}>
                Dashboard
              </Link>
            ) : (
              <>
                <Link href="/login" className={`hidden sm:inline ${NAV_LINK}`}>
                  Sign in
                </Link>
                <a href="/#contact" className={NAV_LINK}>
                  Contact
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
  const { brand } = siteConfig;
  return (
    <footer className="border-t border-zinc-200 bg-white text-ink">
      <div className={`${WRAP} py-12`}>
        <div className="grid gap-8 sm:grid-cols-[1fr_auto]">
          <div className="max-w-sm">
            <Link href="/" className="font-display text-[1.5rem] leading-none">
              {brand.name}
            </Link>
            <p className="mt-3 text-[14px] leading-relaxed text-zinc-600">{brand.footerBlurb}</p>
          </div>
          <ul className="flex flex-col gap-2 text-[14px] sm:text-right">
            <li>
              <a href={`mailto:${brand.email}`} className="text-zinc-700 transition-colors hover:text-ink">
                {brand.email}
              </a>
            </li>
            {brand.socials.map((s) => (
              <li key={s.label}>
                <a href={s.href} className="text-zinc-700 transition-colors hover:text-ink">
                  {s.label}
                </a>
              </li>
            ))}
          </ul>
        </div>
        <div className="mt-10 flex flex-col gap-2 border-t border-zinc-200 pt-6 text-[13px] text-zinc-500 sm:flex-row sm:justify-between">
          <span>
            © {new Date().getFullYear()} {brand.copyrightName}
          </span>
          <span>
            Built with{" "}
            <a href="https://pylonsync.com" className="text-zinc-700 hover:text-ink">
              Pylon
            </a>
          </span>
        </div>
      </div>
    </footer>
  );
}
