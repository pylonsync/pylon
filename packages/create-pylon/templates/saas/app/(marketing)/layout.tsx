import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { ChevronDown, Menu } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { PRODUCTS } from "@/lib/products";
import { SOLUTIONS, RESOURCES, COMPANY, COMPARISONS } from "@/lib/site";
import { siteConfig } from "@/lib/site.config";

// `(marketing)` is a ROUTE GROUP: the parens segment is stripped from every
// URL (so `(marketing)/page.tsx` still serves `/`), and this layout wraps
// only the pages inside the group with the marketing nav and footer. The
// other sections bring their own layouts: `(auth)` the split-screen frame,
// `dashboard/` the sidebar shell.
//
// `auth.user_id` is resolved on the server from the session cookie before any
// HTML is sent, so the nav renders the right buttons on the first byte.
interface LayoutProps {
  children: React.ReactNode;
  auth: PageAuth;
}

type MenuItem = { icon: LucideIcon; title: string; desc: string; href: string };

// Derived from the product list, so the menu and the /products/[slug] pages
// can't drift.
const PRODUCT_MENU: MenuItem[] = PRODUCTS.map((p) => ({
  icon: p.icon,
  title: p.title,
  desc: p.tagline,
  href: `/products/${p.slug}`,
}));

const RESOURCES_MENU: MenuItem[] = siteConfig.resourcesMenu;

// A hover/focus dropdown in pure CSS (`group`), so the nav stays a server
// component with no client JS.
function NavDropdown({ label, items }: { label: string; items: MenuItem[] }) {
  return (
    <div className="group relative">
      <button
        type="button"
        aria-haspopup="menu"
        className="flex items-center gap-1 rounded-full px-3 py-1.5 text-[13.5px] text-zinc-600 transition-colors hover:text-zinc-950 group-hover:text-zinc-950"
      >
        {label}
        <ChevronDown className="size-3.5 text-zinc-400 transition-transform duration-200 group-hover:rotate-180" />
      </button>
      {/* `pt-2` (padding, not margin) bridges the gap so hover doesn't drop. */}
      <div className="invisible absolute left-0 top-full z-40 w-[340px] translate-y-1 pt-2 opacity-0 transition-all duration-200 ease-[cubic-bezier(0.32,0.72,0,1)] group-hover:visible group-hover:translate-y-0 group-hover:opacity-100 group-focus-within:visible group-focus-within:translate-y-0 group-focus-within:opacity-100">
        <div className="rounded-2xl bg-white p-1.5 shadow-[0_0_0_1px_rgba(0,0,0,0.06),0_24px_60px_-20px_rgba(0,0,0,0.25)]">
          {items.map((it) => (
            <a
              key={it.title}
              href={it.href}
              className="flex items-start gap-3 rounded-xl p-2.5 transition-colors hover:bg-zinc-50"
            >
              <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-white text-zinc-700 shadow-[0_0_0_1px_rgba(0,0,0,0.07)]">
                <it.icon className="size-4" strokeWidth={1.75} />
              </span>
              <span className="min-w-0">
                <span className="block text-[13.5px] font-medium text-zinc-900">{it.title}</span>
                <span className="block text-[12.5px] leading-snug text-zinc-500">{it.desc}</span>
              </span>
            </a>
          ))}
        </div>
      </div>
    </div>
  );
}

// Below md the desktop nav is hidden and this menu is the way to reach the
// pages. Native `<details>`, so it needs no client JS; a link click does a
// full navigation, which closes it.
function MobileNav({ signedIn }: { signedIn: boolean }) {
  return (
    <details className="md:hidden">
      <summary
        aria-label="Open menu"
        className="flex size-9 cursor-pointer select-none list-none items-center justify-center rounded-full text-zinc-700 transition-colors marker:hidden hover:bg-zinc-100 [&::-webkit-details-marker]:hidden"
      >
        <Menu className="size-[18px]" strokeWidth={1.75} />
      </summary>
      <div className="fixed inset-x-0 top-14 z-40 max-h-[calc(100dvh-3.5rem)] overflow-y-auto border-b border-zinc-200 bg-white shadow-[0_24px_48px_-24px_rgba(0,0,0,0.25)]">
        <div className="mx-auto max-w-6xl space-y-6 px-4 py-6">
          <MobileGroup title="Product" items={PRODUCT_MENU} />
          <MobileGroup title="Resources" items={RESOURCES_MENU} />
          <a
            href="/pricing"
            className="block rounded-lg px-2 py-2 text-[15px] font-medium text-zinc-800 transition-colors hover:bg-zinc-50"
          >
            Pricing
          </a>
          <div className="flex flex-col gap-2 border-t border-zinc-100 pt-5">
            {signedIn ? (
              <a
                href="/dashboard"
                className="inline-flex h-11 items-center justify-center rounded-full bg-zinc-950 text-[14px] font-medium text-white"
              >
                Open dashboard
              </a>
            ) : (
              <>
                <a
                  href="/signup"
                  className="inline-flex h-11 items-center justify-center rounded-full bg-zinc-950 text-[14px] font-medium text-white"
                >
                  Get started
                </a>
                <a
                  href="/login"
                  className="inline-flex h-11 items-center justify-center rounded-full text-[14px] font-medium text-zinc-900 ring-1 ring-zinc-200"
                >
                  Log in
                </a>
              </>
            )}
          </div>
        </div>
      </div>
    </details>
  );
}

function MobileGroup({ title, items }: { title: string; items: MenuItem[] }) {
  return (
    <div>
      <div className="mb-1 px-2 text-[13px] font-medium text-zinc-400">{title}</div>
      <div className="flex flex-col">
        {items.map((it) => (
          <a
            key={it.href}
            href={it.href}
            className="flex items-center gap-3 rounded-lg px-2 py-2 text-[15px] text-zinc-800 transition-colors hover:bg-zinc-50"
          >
            <it.icon className="size-4 text-zinc-500" strokeWidth={1.75} />
            {it.title}
          </a>
        ))}
      </div>
    </div>
  );
}

function Logo() {
  return (
    <Link href="/" className="flex items-center gap-2">
      <span className="flex size-6 items-center justify-center rounded-[7px] bg-zinc-950 text-[13px] font-bold text-white">
        {siteConfig.brand.letter}
      </span>
      <span className="text-[15px] font-semibold tracking-tight text-zinc-950">{siteConfig.brand.name}</span>
    </Link>
  );
}

// `<main>` is full-bleed; each page owns its container.
export default function MarketingLayout({ children, auth }: LayoutProps) {
  const signedIn = Boolean(auth?.user_id);
  return (
    <>
      <header className="sticky top-0 z-30 border-b border-zinc-950/[0.06] bg-white/75 backdrop-blur-xl">
        <div className="mx-auto flex h-14 max-w-6xl items-center justify-between px-4 sm:px-6">
          <div className="flex items-center gap-6">
            <Logo />
            <nav className="hidden items-center md:flex">
              <NavDropdown label="Product" items={PRODUCT_MENU} />
              <NavDropdown label="Resources" items={RESOURCES_MENU} />
              <Link
                href="/pricing"
                className="rounded-full px-3 py-1.5 text-[13.5px] text-zinc-600 transition-colors hover:text-zinc-950"
              >
                Pricing
              </Link>
            </nav>
          </div>
          <nav className="flex items-center gap-1.5">
            {signedIn ? (
              <Link
                href="/dashboard"
                className="inline-flex h-8 items-center rounded-full bg-zinc-950 px-3.5 text-[13px] font-medium text-white transition-colors hover:bg-zinc-800"
              >
                Open dashboard
              </Link>
            ) : (
              <>
                <Link
                  href="/login"
                  className="hidden h-8 items-center rounded-full px-3 text-[13px] font-medium text-zinc-600 transition-colors hover:text-zinc-950 sm:inline-flex"
                >
                  Log in
                </Link>
                <Link
                  href="/signup"
                  className="inline-flex h-8 items-center rounded-full bg-zinc-950 px-3.5 text-[13px] font-medium text-white transition-colors hover:bg-zinc-800"
                >
                  Get started
                </Link>
              </>
            )}
            <MobileNav signedIn={signedIn} />
          </nav>
        </div>
      </header>

      <main className="flex-1">{children}</main>

      <SiteFooter />
    </>
  );
}

// Footer link groups, derived from the same data the pages use, so every
// link resolves to a real route.
type FooterGroupData = { title: string; links: { label: string; href: string }[] };

const FOOTER_GROUPS: FooterGroupData[] = [
  {
    title: "Product",
    links: [
      ...PRODUCTS.map((p) => ({ label: p.title, href: `/products/${p.slug}` })),
      { label: "Pricing", href: "/pricing" },
    ],
  },
  {
    title: "Solutions",
    links: [
      ...SOLUTIONS.map((s) => ({ label: s.navLabel, href: `/solutions/${s.slug}` })),
      ...COMPARISONS.map((c) => ({ label: c.navLabel, href: `/compare/${c.slug}` })),
    ],
  },
  {
    title: "Resources",
    links: RESOURCES.map((r) => ({ label: r.navLabel, href: `/resources/${r.slug}` })),
  },
  {
    title: "Company",
    links: COMPANY.map((c) => ({ label: c.navLabel, href: `/company/${c.slug}` })),
  },
];

function SiteFooter() {
  return (
    <footer className="border-t border-zinc-200/70 bg-white">
      <div className="mx-auto grid max-w-6xl gap-12 px-4 py-14 sm:px-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
        <div className="max-w-xs">
          <Logo />
          <p className="mt-4 text-[13.5px] leading-relaxed text-zinc-500">{siteConfig.brand.footerBlurb}</p>
          <div className="mt-5 flex items-center gap-4">
            {siteConfig.brand.socials.map((s) => (
              <a
                key={s.label}
                href={s.href}
                aria-label={s.label}
                className="text-zinc-400 transition-colors hover:text-zinc-950"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                  <path d={s.path} />
                </svg>
              </a>
            ))}
          </div>
        </div>
        <div className="grid grid-cols-2 gap-x-6 gap-y-10 sm:grid-cols-4">
          {FOOTER_GROUPS.map((g) => (
            <div key={g.title}>
              <div className="text-[13px] font-medium text-zinc-950">{g.title}</div>
              <ul className="mt-3.5 space-y-2.5">
                {g.links.map((l) => (
                  <li key={l.href}>
                    <Link href={l.href} className="text-[13px] text-zinc-500 transition-colors hover:text-zinc-950">
                      {l.label}
                    </Link>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
      </div>
      <div className="mx-auto max-w-6xl border-t border-zinc-200/70 px-4 py-6 text-[12.5px] text-zinc-400 sm:px-6">
        © {new Date().getFullYear()} {siteConfig.brand.copyrightName}
      </div>
    </footer>
  );
}
