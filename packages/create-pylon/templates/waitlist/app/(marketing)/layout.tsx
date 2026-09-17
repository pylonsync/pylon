import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";
import { WRAP } from "@/components/marketing";

// `(marketing)` is a ROUTE GROUP: the parens segment is stripped from every
// URL (so `(marketing)/page.tsx` still serves `/`), and this layout wraps only
// the pages inside the group. A one-line nav up top and a one-line footer
// below, both driven by lib/site.config.ts. /login and /dashboard sit outside
// the group and render bare in the root shell. `auth.user_id` is resolved
// server-side from the session cookie before any HTML is sent, so the nav
// shows "Dashboard" once the owner is signed in and "Sign in" otherwise.
interface LayoutProps {
  children: React.ReactNode;
  auth: PageAuth;
}

export default function MarketingLayout({ children, auth }: LayoutProps) {
  // A guest session (minted by <EnsureGuest> for the live counter) has a
  // `guest_…` user id. That is an anonymous visitor, not the signed-in owner,
  // so it must not flip the nav to "Dashboard".
  const signedIn = Boolean(auth?.user_id && !auth.user_id.startsWith("guest_"));
  const { brand } = siteConfig;

  return (
    <div className="flex min-h-screen flex-col bg-ink text-chalk">
      <header>
        <div className={`${WRAP} flex h-16 items-center justify-between`}>
          <Link href="/" className="font-display text-[17px] font-bold tracking-tight text-chalk">
            {brand.name}
          </Link>
          <nav>
            <Link
              href={signedIn ? "/dashboard" : "/login"}
              className="text-[14px] text-chalk-2 transition-colors hover:text-chalk"
            >
              {signedIn ? "Dashboard" : "Sign in"}
            </Link>
          </nav>
        </div>
      </header>

      <main className="flex-1">{children}</main>

      <SiteFooter />
    </div>
  );
}

function SiteFooter() {
  const { brand } = siteConfig;
  return (
    <footer className="border-t border-line">
      <div
        className={`${WRAP} flex flex-col gap-3 py-6 font-mono-ui text-[12px] text-chalk-2 sm:flex-row sm:items-center sm:justify-between`}
      >
        <span>
          {brand.copyrightName} {new Date().getFullYear()}
        </span>
        <div className="flex items-center gap-5">
          {brand.socials.map((s) => (
            <a key={s.label} href={s.href} className="transition-colors hover:text-chalk">
              {s.label}
            </a>
          ))}
          <a href={`mailto:${brand.email}`} className="transition-colors hover:text-chalk">
            {brand.email}
          </a>
          <a href="https://pylonsync.com" className="transition-colors hover:text-chalk">
            Built with Pylon
          </a>
        </div>
      </div>
    </footer>
  );
}
