import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";
import { CartButton } from "@/components/cart-button";
import { WRAP } from "@/components/marketing";

// `(marketing)` is a ROUTE GROUP: the parens segment is stripped from every
// URL (so `(marketing)/page.tsx` still serves `/`), and this layout wraps only
// the pages inside the group: an announcement bar, a sticky nav, and a footer,
// all driven by lib/site.config.ts. /login and /dashboard sit outside the
// group and render bare in the root shell. `auth.user_id` is resolved
// server-side from the session cookie before any HTML is sent, so the nav
// shows "Dashboard" once the owner is signed in and "Sign in" otherwise.
interface LayoutProps {
  children: React.ReactNode;
  auth: PageAuth;
}

const NAV_LINKS = [
  { label: "Shop", href: "/#shop" },
  { label: "Studio", href: "/#studio" },
  { label: "Shipping", href: "/#shipping" },
];

const NAV_LINK =
  "relative text-[14px] text-ink/80 transition-colors duration-300 hover:text-ink after:absolute after:inset-x-0 after:-bottom-1 after:h-px after:origin-left after:scale-x-0 after:bg-ink after:transition-transform after:duration-500 after:ease-out-quint hover:after:scale-x-100";

export default function MarketingLayout({ children, auth }: LayoutProps) {
  // A guest session (minted by <EnsureGuest> for the live stock grid) has a
  // `guest_…` user id. That is an anonymous visitor, not the signed-in owner,
  // so it does not flip the nav to "Dashboard".
  const signedIn = Boolean(auth?.user_id && !auth.user_id.startsWith("guest_"));
  const { brand } = siteConfig;

  return (
    <div className="grain flex min-h-screen flex-col bg-warm-white text-ink">
      <div className="bg-espresso text-warm-white/85">
        <p className={`${WRAP} flex min-h-9 items-center justify-center py-2 text-center text-[12px] leading-snug tracking-[0.01em] sm:text-[12.5px]`}>
          {brand.announcement}
        </p>
      </div>

      <header className="sticky top-0 z-30 border-b border-ink/10 bg-warm-white/85 backdrop-blur-md">
        <div className={`${WRAP} grid h-16 grid-cols-[1fr_auto_1fr] items-center`}>
          <nav className="hidden items-center gap-8 md:flex" aria-label="Sections">
            {NAV_LINKS.map((l) => (
              <a key={l.href} href={l.href} className={NAV_LINK}>
                {l.label}
              </a>
            ))}
          </nav>
          <Link
            href="/"
            className="font-display col-start-1 justify-self-start text-[25px] leading-none tracking-[-0.015em] text-ink md:col-start-2 md:justify-self-center"
          >
            {brand.name}
          </Link>
          <div className="col-start-3 flex items-center justify-end gap-1 sm:gap-3">
            {signedIn ? (
              <Link href="/dashboard" className={`${NAV_LINK} hidden sm:inline`}>
                Dashboard
              </Link>
            ) : (
              <Link href="/login" className={`${NAV_LINK} hidden sm:inline`}>
                Sign in
              </Link>
            )}
            {/* A client island: it reads the shared cart store so its count
                matches the grid. */}
            <CartButton />
          </div>
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
    <footer className="bg-espresso text-warm-white">
      <div className={`${WRAP} pt-20 pb-10`}>
        <div className="grid gap-12 md:grid-cols-[1.4fr_1fr_1fr_1fr]">
          <div className="max-w-xs">
            <Link href="/" className="font-display text-[32px] leading-none tracking-[-0.015em]">
              {brand.name}
            </Link>
            <p className="mt-4 text-[14px] leading-relaxed text-warm-white/60">{brand.footerBlurb}</p>
          </div>
          <FooterColumn title="Shop">
            {NAV_LINKS.map((l) => (
              <li key={l.href}>
                <a href={l.href} className="transition-colors hover:text-warm-white">
                  {l.label}
                </a>
              </li>
            ))}
          </FooterColumn>
          <FooterColumn title="Studio">
            {brand.address.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </FooterColumn>
          <FooterColumn title="Contact">
            <li>
              <a href={`mailto:${brand.email}`} className="transition-colors hover:text-warm-white">
                {brand.email}
              </a>
            </li>
            {brand.socials.map((s) => (
              <li key={s.label}>
                <a href={s.href} className="inline-flex items-center gap-2 transition-colors hover:text-warm-white">
                  <svg width="15" height="15" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                    <path d={s.path} />
                  </svg>
                  {s.label}
                </a>
              </li>
            ))}
          </FooterColumn>
        </div>
        <div className="mt-20 flex flex-col items-start justify-between gap-3 border-t border-warm-white/10 pt-6 text-[12.5px] text-warm-white/50 sm:flex-row sm:items-center">
          <span>
            © {new Date().getFullYear()} {brand.copyrightName}
          </span>
          <span>
            Built with{" "}
            <a href="https://pylonsync.com" className="text-warm-white/80 transition-colors hover:text-warm-white">
              Pylon
            </a>
          </span>
        </div>
      </div>
    </footer>
  );
}

function FooterColumn({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h3 className="text-[13px] font-medium text-warm-white">{title}</h3>
      <ul className="mt-4 space-y-2.5 text-[14px] text-warm-white/60">{children}</ul>
    </div>
  );
}
