import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";
import { WRAP } from "@/components/marketing";

// `(marketing)` is a ROUTE GROUP: the parens segment is stripped from every
// URL (so `(marketing)/page.tsx` still serves `/`), and this layout wraps
// only the pages inside the group: a one-line nav and a footer, both driven by
// lib/site.config.ts. Routes outside the group (/login, /dashboard) render bare
// in the root shell. `auth.user_id` is resolved server-side from the session
// cookie before any HTML is sent, so the nav shows "Dashboard" once the owner
// is signed in and "Sign in" otherwise.
interface LayoutProps {
  children: React.ReactNode;
  auth: PageAuth;
}

const NAV_LINK = "text-[14px] text-zinc-600 hover:text-zinc-900";

export default function MarketingLayout({ children, auth }: LayoutProps) {
  // A guest session (minted by <EnsureGuest> for the live search) has a
  // `guest_…` user id. That is an anonymous visitor, not the signed-in owner.
  const signedIn = Boolean(auth?.user_id && !auth.user_id.startsWith("guest_"));
  const { brand, intro } = siteConfig;

  return (
    <>
      <header className="border-b border-zinc-200 bg-white">
        <div className={`${WRAP} flex h-12 items-center justify-between`}>
          <Link href="/" className="text-[15px] font-semibold text-zinc-900">
            {brand.name}
          </Link>
          <nav className="flex items-center gap-5">
            <a href="/#browse" className={NAV_LINK}>
              Browse
            </a>
            <Link href="/submit" className={NAV_LINK}>
              {intro.ctaLabel}
            </Link>
            {signedIn ? (
              <Link href="/dashboard" className={NAV_LINK}>
                Dashboard
              </Link>
            ) : (
              <Link href="/login" className={NAV_LINK}>
                Sign in
              </Link>
            )}
          </nav>
        </div>
      </header>

      <main className="flex-1 bg-white">{children}</main>

      <SiteFooter />
    </>
  );
}

function SiteFooter() {
  const { brand } = siteConfig;
  return (
    <footer className="border-t border-zinc-200 bg-white">
      <div className={`${WRAP} flex flex-col gap-3 py-8 text-[13px] text-zinc-500 sm:flex-row sm:items-center sm:justify-between`}>
        <p className="max-w-md">{brand.footerBlurb}</p>
        <div className="flex items-center gap-4">
          {brand.socials.map((s) => (
            <a key={s.label} href={s.href} className="hover:text-zinc-900">
              {s.label}
            </a>
          ))}
          <a href={`mailto:${brand.email}`} className="hover:text-zinc-900">
            Email
          </a>
          <span className="font-mono text-[12px] text-zinc-400">
            © {new Date().getFullYear()} {brand.copyrightName}
          </span>
        </div>
      </div>
    </footer>
  );
}
