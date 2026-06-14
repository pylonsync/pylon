import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { PRODUCTS } from "@/lib/products";
import { SOLUTIONS, RESOURCES, COMPANY, COMPARISONS } from "@/lib/site";

// A layout receives the page props plus `children`. `auth.user_id` is null for
// anonymous visitors and the signed-in user's id otherwise — resolved
// server-side from the session cookie before any HTML is sent, so the nav
// renders the right links on the first byte (no flash, no client fetch).
interface LayoutProps {
  children: React.ReactNode;
  url: string;
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
  { icon: "▢", title: "Docs", desc: "Set up and use Acme.", href: "/resources/docs" },
  { icon: "✎", title: "Guides", desc: "Playbooks and walkthroughs.", href: "/resources/guides" },
  { icon: "✦", title: "Changelog", desc: "What's new in Acme.", href: "/resources/changelog" },
  { icon: "⌘", title: "API reference", desc: "Build on the Acme API.", href: "/resources/api" },
  { icon: "◈", title: "Compare", desc: "See how Acme stacks up.", href: `/compare/${COMPARISONS[0].slug}` },
];

// A hover/focus dropdown — pure CSS via `group`, so the nav stays a server
// component (no client JS). The panel opens on hover and on keyboard focus.
function NavDropdown({ label, items }: { label: string; items: MenuItem[] }) {
  return (
    <div className="group relative">
      <button
        type="button"
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

// The root layout wraps every page: a marketing nav up top, a fat footer below.
// `<main>` is full-bleed — each page owns its own container so the landing page
// can run edge-to-edge while the app pages stay centered. Rebrand "Acme" to
// your product.
export default function RootLayout({ children, auth }: LayoutProps) {
  const signedIn = Boolean(auth?.user_id);
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        {/* No <title> here on purpose — each page's exported `metadata` /
            `generateMetadata` sets it. A hardcoded title in the layout would
            render first and win over the page's, so every tab would read
            "Acme". */}
        {/* Inter — the marketing pages look best in a clean grotesk. Swap for
            your own font or drop this link to fall back to the system stack. */}
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link
          rel="preconnect"
          href="https://fonts.gstatic.com"
          crossOrigin="anonymous"
        />
        <link
          rel="stylesheet"
          href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap"
        />
        {/* Tailwind is compiled by Pylon from app/globals.css and the
            stylesheet link is injected here automatically. */}
      </head>
      <body className="flex min-h-screen flex-col bg-background text-foreground antialiased">
        <header className="sticky top-0 z-30 bg-white/80 backdrop-blur">
          <div className="mx-auto flex h-14 max-w-5xl items-center justify-between px-6">
            <div className="flex items-center gap-8">
              <Link href="/" className="flex items-center gap-2">
                <span className="flex size-6 items-center justify-center rounded-[7px] bg-zinc-900 text-[13px] font-bold text-white">
                  A
                </span>
                <span className="text-[15px] font-semibold tracking-tight text-zinc-900">
                  Acme
                </span>
              </Link>
              <nav className="hidden items-center gap-6 md:flex">
                <NavDropdown label="Product" items={PRODUCT_MENU} />
                <NavDropdown label="Resources" items={RESOURCES_MENU} />
                <Link
                  href="/#pricing"
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
      </body>
    </html>
  );
}

// Footer columns, derived from the same data the pages use so every link
// resolves to a real route.
type FooterLink = { label: string; href: string };
const FOOTER_COLS: { title: string; links: FooterLink[] }[] = [
  {
    title: "Product",
    links: [
      ...PRODUCTS.map((p) => ({ label: p.title, href: `/products/${p.slug}` })),
      { label: "Pricing", href: "/#pricing" },
    ],
  },
  {
    title: "Solutions",
    links: SOLUTIONS.map((s) => ({
      label: s.navLabel,
      href: `/solutions/${s.slug}`,
    })),
  },
  {
    title: "Resources",
    links: RESOURCES.map((r) => ({
      label: r.navLabel,
      href: `/resources/${r.slug}`,
    })),
  },
  {
    title: "Compare",
    links: COMPARISONS.map((c) => ({
      label: c.navLabel,
      href: `/compare/${c.slug}`,
    })),
  },
  {
    title: "Company",
    links: COMPANY.map((c) => ({
      label: c.navLabel,
      href: `/company/${c.slug}`,
    })),
  },
];

function SiteFooter() {
  return (
    <footer className="border-t border-zinc-200/70 bg-white">
      <div className="mx-auto max-w-5xl px-6 py-14">
        <div className="grid gap-10 sm:grid-cols-2 lg:grid-cols-[1.4fr_repeat(5,1fr)]">
          {/* Brand */}
          <div>
            <div className="flex items-center gap-2">
              <span className="flex size-6 items-center justify-center rounded-[7px] bg-zinc-900 text-[13px] font-bold text-white">
                A
              </span>
              <span className="text-[15px] font-semibold tracking-tight text-zinc-900">
                Acme
              </span>
            </div>
            <p className="mt-4 max-w-[26ch] text-[13px] leading-relaxed text-zinc-500">
              The workspace where your team plans, builds, and ships together.
            </p>
            <div className="mt-5 flex items-center gap-3 text-zinc-400">
              {["X", "in", "GH"].map((s) => (
                <span
                  key={s}
                  className="flex size-7 items-center justify-center rounded-md border border-zinc-200 text-[11px] font-medium"
                >
                  {s}
                </span>
              ))}
            </div>
          </div>
          {/* Link columns */}
          {FOOTER_COLS.map((col) => (
            <div key={col.title}>
              <div className="font-mono text-[10.5px] uppercase tracking-[0.12em] text-zinc-400">
                {col.title}
              </div>
              <div className="mt-4 flex flex-col gap-2.5">
                {col.links.map((l) => (
                  <Link
                    key={l.href}
                    href={l.href}
                    className="text-[13px] text-zinc-500 transition-colors hover:text-zinc-900"
                  >
                    {l.label}
                  </Link>
                ))}
              </div>
            </div>
          ))}
        </div>
        <div className="mt-12 flex flex-col items-start justify-between gap-3 border-t border-zinc-200/70 pt-6 text-[12px] text-zinc-400 sm:flex-row sm:items-center">
          <span>© {new Date().getFullYear()} Acme, Inc.</span>
          <span>
            Built with{" "}
            <a
              href="https://pylonsync.com"
              className="font-medium text-zinc-600 hover:text-zinc-900"
            >
              Pylon
            </a>
          </span>
        </div>
      </div>
    </footer>
  );
}
