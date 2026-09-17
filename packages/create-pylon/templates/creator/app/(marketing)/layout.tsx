import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";
import { WRAP } from "@/components/marketing";

// `(marketing)` is a ROUTE GROUP: the parens segment is stripped from every
// URL (so `(marketing)/page.tsx` still serves `/`), and this layout wraps
// only the pages inside the group: the name up top, a footer below, both
// driven by lib/site.config.ts. Routes outside the group (/login, /dashboard)
// render bare in the root shell. `auth.user_id` is resolved server-side from
// the session cookie before any HTML is sent, so the nav shows "Dashboard"
// once the owner is signed in and "Sign in" otherwise. No flash, no client
// fetch.
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
    <div className="font-display flex min-h-screen flex-1 flex-col bg-paper text-ink">
      <header>
        <div className={`${WRAP} flex h-16 items-center justify-between`}>
          <Link href="/" className="text-[17px] font-medium text-ink">
            {brand.name}
          </Link>
          <nav>
            {signedIn ? (
              <Link href="/dashboard" className="text-[16px] text-ink-2 hover:text-ink">
                Dashboard
              </Link>
            ) : (
              <Link href="/login" className="text-[16px] text-ink-2 hover:text-ink">
                Sign in
              </Link>
            )}
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
    <footer>
      <div className={`${WRAP} border-t border-rule py-10 text-[15px] text-ink-2`}>
        <p className="max-w-md leading-relaxed">{brand.footerBlurb}</p>
        <ul className="mt-5 flex flex-wrap gap-x-5 gap-y-2">
          {brand.socials.map((s) => (
            <li key={s.label}>
              <a href={s.href} className="underline decoration-rule underline-offset-4 hover:text-ink">
                {s.label}
              </a>
            </li>
          ))}
          <li>
            <a href={`mailto:${brand.email}`} className="underline decoration-rule underline-offset-4 hover:text-ink">
              {brand.email}
            </a>
          </li>
        </ul>
        <div className="mt-8 flex flex-col gap-1 text-[14px] sm:flex-row sm:justify-between">
          <span>
            © {new Date().getFullYear()} {brand.copyrightName}
          </span>
          <span>
            Built with{" "}
            <a href="https://pylonsync.com" className="underline decoration-rule underline-offset-4 hover:text-ink">
              Pylon
            </a>
          </span>
        </div>
      </div>
    </footer>
  );
}
