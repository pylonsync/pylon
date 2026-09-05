import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { PRODUCTS } from "@/lib/products";
import { SOLUTIONS, RESOURCES, COMPANY, COMPARISONS } from "@/lib/site";
import { siteConfig } from "@/lib/site.config";

// `(marketing)` is a ROUTE GROUP: the parens segment is stripped from every
// URL (so `(marketing)/page.tsx` still serves `/`), and this layout wraps
// only the pages inside the group — the marketing nav up top, a fat footer
// below. The other sections bring their own layouts: `(auth)` the split-screen
// frame, `dashboard/` the sidebar shell.
//
// A layout receives the page props plus `children`. `auth.user_id` is null for
// anonymous visitors and the signed-in user's id otherwise — resolved
// server-side from the session cookie before any HTML is sent, so the nav
// renders the right links on the first byte (no flash, no client fetch).
interface LayoutProps {
  children: React.ReactNode;
  auth: PageAuth;
}

// Dropdown menu data. Each item is icon + title + blurb + anchor. Swap the
// hrefs for real routes as you add pages.
type MenuItem = { icon: string; title: string; desc: string; href: string };

// Derived from the shared product list so the menu and the /products/[slug]
// pages can never drift. Each item deep-links to that product's own page.
const PRODUCT_MENU: MenuItem[] = PRODUCTS.map((p) => ({
  icon: p.icon,
  title: p.title,
  desc: p.tagline,
  href: `/products/${p.slug}`,
}));

const RESOURCES_MENU: MenuItem[] = [
  { icon: "▢", title: "Docs", desc: `Set up and use ${siteConfig.brand.name}.`, href: "/resources/docs" },
  { icon: "✎", title: "Guides", desc: "Playbooks and walkthroughs.", href: "/resources/guides" },
  { icon: "✦", title: "Changelog", desc: `What's new in ${siteConfig.brand.name}.`, href: "/resources/changelog" },
  { icon: "⌘", title: "API reference", desc: `Build on the ${siteConfig.brand.name} API.`, href: "/resources/api" },
  { icon: "◈", title: "Compare", desc: `See how ${siteConfig.brand.name} stacks up.`, href: `/compare/${COMPARISONS[0].slug}` },
];

// A hover/focus dropdown — pure CSS via `group`, so the nav stays a server
// component (no client JS). The panel opens on hover and on keyboard focus.
function NavDropdown({ label, items }: { label: string; items: MenuItem[] }) {
  return (
    <div className="group relative">
      <button
        type="button"
        aria-haspopup="menu"
        className="flex items-center gap-1 text-[13.5px] text-zinc-600 transition-colors hover:text-zinc-900 group-hover:text-zinc-900"
      >
        {label}
        <Chevron />
      </button>
      {/* `pt-3` (padding, not margin) bridges the gap so hover doesn't drop. */}
      <div className="invisible absolute left-0 top-full z-40 w-[340px] translate-y-1 pt-3 opacity-0 transition-all duration-150 group-hover:visible group-hover:translate-y-0 group-hover:opacity-100 group-focus-within:visible group-focus-within:translate-y-0 group-focus-within:opacity-100">
        <div className="rounded-2xl border border-zinc-200 bg-white p-2 shadow-[0_24px_60px_-25px_rgba(0,0,0,0.25)]">
          {items.map((it) => (
            <a
              key={it.title}
              href={it.href}
              className="flex items-start gap-3 rounded-xl p-3 transition-colors hover:bg-zinc-50"
            >
              <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-brand-soft text-[13px] text-brand">
                {it.icon}
              </span>
              <span className="min-w-0">
                <span className="block text-[13.5px] font-medium text-zinc-900">
                  {it.title}
                </span>
                <span className="block text-[12.5px] leading-snug text-zinc-500">
                  {it.desc}
                </span>
              </span>
            </a>
          ))}
        </div>
      </div>
    </div>
  );
}

function Chevron() {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      fill="none"
      aria-hidden
      className="text-zinc-400 transition-transform duration-200 group-hover:rotate-180"
    >
      <path
        d="M2 3.5L5 6.5L8 3.5"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

// Mobile menu — the desktop nav is `hidden md:flex`, so on phones this is the
// only way to reach Product / Resources / Pricing. Pure-CSS via native
// `<details>` (no client JS, same pattern as the FAQ + user menu); plain `<a>`
// links do a full navigation, which closes the open panel. md:hidden so it
// never shows alongside the desktop nav.
function MobileNav({ signedIn }: { signedIn: boolean }) {
  return (
    <details className="md:hidden">
      <summary
        aria-label="Open menu"
        className="flex size-9 cursor-pointer select-none list-none items-center justify-center rounded-md text-zinc-700 transition-colors marker:hidden hover:bg-zinc-100 [&::-webkit-details-marker]:hidden"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden>
          <path d="M3 6h18M3 12h18M3 18h18" />
        </svg>
      </summary>
      <div className="fixed inset-x-0 top-14 z-40 max-h-[calc(100vh-3.5rem)] overflow-y-auto border-b border-zinc-200 bg-white shadow-[0_24px_48px_-24px_rgba(0,0,0,0.25)]">
        <div className="mx-auto max-w-5xl space-y-6 px-6 py-6">
          <MobileGroup title="Product" items={PRODUCT_MENU} />
          <MobileGroup title="Resources" items={RESOURCES_MENU} />
          <div className="flex flex-col">
            <a href="/#pricing" className="rounded-lg px-2 py-2 text-[14px] font-medium text-zinc-700 transition-colors hover:bg-zinc-50">
              Pricing
            </a>
            <a href="/#customers" className="rounded-lg px-2 py-2 text-[14px] font-medium text-zinc-700 transition-colors hover:bg-zinc-50">
              Customers
            </a>
          </div>
          <div className="flex flex-col gap-2 border-t border-zinc-100 pt-5">
            {signedIn ? (
              <a href="/dashboard" className="inline-flex h-10 items-center justify-center rounded-full bg-zinc-900 text-[14px] font-medium text-white transition-colors hover:bg-zinc-700">
                Open dashboard
              </a>
            ) : (
              <>
                <a href="/login" className="inline-flex h-10 items-center justify-center rounded-full border border-zinc-300 text-[14px] font-medium text-zinc-900 transition-colors hover:bg-zinc-50">
                  Log in
                </a>
                <a href="/signup" className="inline-flex h-10 items-center justify-center rounded-full bg-zinc-900 text-[14px] font-medium text-white transition-colors hover:bg-zinc-700">
                  Get started
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
      <div className="mb-1.5 px-2 text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
        {title}
      </div>
      <div className="flex flex-col">
        {items.map((it) => (
          <a
            key={it.href}
            href={it.href}
            className="flex items-center gap-3 rounded-lg px-2 py-2 text-[14px] text-zinc-700 transition-colors hover:bg-zinc-50"
          >
            <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-brand-soft text-[12px] text-brand">
              {it.icon}
            </span>
            {it.title}
          </a>
        ))}
      </div>
    </div>
  );
}

// `<main>` is full-bleed — each page owns its own container so the landing
// page can run edge-to-edge while inner pages stay centered. Rebrand "Acme"
// to your product.
export default function MarketingLayout({ children, auth }: LayoutProps) {
  const signedIn = Boolean(auth?.user_id);
  return (
    <>
      <header className="sticky top-0 z-30 bg-white/80 backdrop-blur">
        <div className="mx-auto flex h-14 max-w-5xl items-center justify-between px-6">
          <div className="flex items-center gap-8">
            <Link href="/" className="flex items-center gap-2">
              <span className="flex size-6 items-center justify-center rounded-[7px] bg-zinc-900 text-[13px] font-bold text-white">
                {siteConfig.brand.letter}
              </span>
              <span className="text-[15px] font-semibold tracking-tight text-zinc-900">
                {siteConfig.brand.name}
              </span>
            </Link>
            <nav className="hidden items-center gap-6 md:flex">
              <NavDropdown label="Product" items={PRODUCT_MENU} />
              <NavDropdown label="Resources" items={RESOURCES_MENU} />
              <Link
                href="/pricing"
                className="text-[13.5px] text-zinc-600 transition-colors hover:text-zinc-900"
              >
                Pricing
              </Link>
              <Link
                href="/#customers"
                className="text-[13.5px] text-zinc-600 transition-colors hover:text-zinc-900"
              >
                Customers
              </Link>
            </nav>
          </div>
          <nav className="flex items-center gap-2">
            <MobileNav signedIn={signedIn} />
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
                  Log in
                </Link>
                <Link
                  href="/signup"
                  className="inline-flex items-center rounded-full bg-zinc-900 px-3.5 py-1.5 text-[13px] font-medium text-white transition-colors hover:bg-zinc-700"
                >
                  Get started
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

// Footer link groups, derived from the same data the pages use so every link
// resolves to a real route. Each link carries a small icon (except Compare).
type FooterLink = { label: string; href: string; icon?: string };
type FooterGroupData = { title: string; links: FooterLink[] };

const SOLUTION_ICONS = ["◆", "◈", "▣", "◎"];
const RESOURCE_ICONS: Record<string, string> = {
  docs: "▢",
  guides: "✎",
  changelog: "✦",
  api: "⌘",
  status: "◉",
};
const COMPANY_ICONS = ["◍", "≣", "◆", "✉", "▢"];

const PRODUCT_GROUP: FooterGroupData = {
  title: "Product",
  links: [
    ...PRODUCTS.map((p) => ({
      label: p.title,
      href: `/products/${p.slug}`,
      icon: p.icon,
    })),
    { label: "Pricing", href: "/#pricing", icon: "◷" },
  ],
};
const SOLUTIONS_GROUP: FooterGroupData = {
  title: "Solutions",
  links: SOLUTIONS.map((s, i) => ({
    label: s.navLabel,
    href: `/solutions/${s.slug}`,
    icon: SOLUTION_ICONS[i] ?? "◆",
  })),
};
const RESOURCES_GROUP: FooterGroupData = {
  title: "Resources",
  links: RESOURCES.map((r) => ({
    label: r.navLabel,
    href: `/resources/${r.slug}`,
    icon: RESOURCE_ICONS[r.slug] ?? "▢",
  })),
};
const COMPANY_GROUP: FooterGroupData = {
  title: "Company",
  links: COMPANY.map((c, i) => ({
    label: c.navLabel,
    href: `/company/${c.slug}`,
    icon: COMPANY_ICONS[i] ?? "◍",
  })),
};
const COMPARE_GROUP: FooterGroupData = {
  title: "Compare",
  links: COMPARISONS.map((c) => ({
    label: c.navLabel,
    href: `/compare/${c.slug}`,
  })),
};

function FooterGroup({
  group,
  className = "",
}: {
  group: FooterGroupData;
  className?: string;
}) {
  return (
    <div className={className}>
      <div className="text-[11px] font-semibold uppercase tracking-wider text-zinc-900">
        {group.title}
      </div>
      <div className="mt-4 flex flex-col gap-2.5">
        {group.links.map((l) => (
          <Link
            key={l.href}
            href={l.href}
            className="group flex items-center gap-2 text-[13px] text-zinc-500 transition-colors hover:text-zinc-900"
          >
            {l.icon ? (
              <span className="flex w-3.5 justify-center text-[11px] text-zinc-400 transition-colors group-hover:text-zinc-600">
                {l.icon}
              </span>
            ) : null}
            {l.label}
          </Link>
        ))}
      </div>
    </div>
  );
}

function SiteFooter() {
  return (
    <footer className="border-t border-zinc-200/70 bg-white">
      <div className="mx-auto max-w-5xl px-6 py-16">
        {/* Brand block */}
        <div className="max-w-xs">
          <Link href="/" className="inline-flex items-center gap-2">
            <span className="flex size-7 items-center justify-center rounded-lg bg-zinc-900 text-sm font-bold text-white">
              {siteConfig.brand.letter}
            </span>
            <span className="text-[15px] font-semibold tracking-tight text-zinc-900">
              {siteConfig.brand.name}
            </span>
          </Link>
          <p className="mt-4 text-[13px] leading-relaxed text-zinc-500">
            {siteConfig.brand.footerBlurb}
          </p>
          <div className="mt-5 flex items-center gap-4">
            {siteConfig.brand.socials.map((s) => (
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
              href={`mailto:${siteConfig.brand.email}`}
              aria-label="Email"
              className="text-zinc-400 transition-colors hover:text-zinc-900"
            >
              <svg
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
              >
                <rect x="3" y="5" width="18" height="14" rx="2" />
                <path d="m3 7 9 6 9-6" />
              </svg>
            </a>
          </div>
        </div>

        {/* Link columns */}
        <div className="mt-12 grid gap-x-8 gap-y-12 sm:grid-cols-2 lg:grid-cols-3">
          <div>
            <FooterGroup group={PRODUCT_GROUP} />
            <FooterGroup group={SOLUTIONS_GROUP} className="mt-10" />
          </div>
          <div>
            <FooterGroup group={RESOURCES_GROUP} />
            <FooterGroup group={COMPANY_GROUP} className="mt-10" />
          </div>
          <div>
            <FooterGroup group={COMPARE_GROUP} />
          </div>
        </div>

        <div className="mt-14 border-t border-zinc-200/70 pt-6 text-[12px] text-zinc-400">
          <span>© {new Date().getFullYear()} {siteConfig.brand.copyrightName}</span>
        </div>
      </div>
    </footer>
  );
}
