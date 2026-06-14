import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";

// A layout receives the page props plus `children`. `auth.user_id` is null for
// anonymous visitors and the signed-in user's id otherwise — resolved
// server-side from the session cookie before any HTML is sent, so the nav
// renders the right links on the first byte (no flash, no client fetch).
interface LayoutProps {
  children: React.ReactNode;
  url: string;
  auth: PageAuth;
}

const NAV_LINKS = [
  { label: "Product", href: "/#product" },
  { label: "Customers", href: "/#customers" },
  { label: "Pricing", href: "/#pricing" },
  { label: "FAQ", href: "/#faq" },
];

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
        <title>Acme</title>
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
        <header className="sticky top-0 z-30 border-b border-zinc-200/70 bg-white/80 backdrop-blur">
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
                {NAV_LINKS.map((l) => (
                  <Link
                    key={l.href}
                    href={l.href}
                    className="text-[13.5px] text-zinc-600 transition-colors hover:text-zinc-900"
                  >
                    {l.label}
                  </Link>
                ))}
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

const FOOTER_COLS: { title: string; links: string[] }[] = [
  {
    title: "Product",
    links: ["Overview", "Projects", "Roadmap", "Updates", "Integrations"],
  },
  {
    title: "Solutions",
    links: ["For startups", "For agencies", "For enterprise", "For teams"],
  },
  {
    title: "Resources",
    links: ["Docs", "Guides", "Changelog", "Status", "API reference"],
  },
  {
    title: "Company",
    links: ["About", "Blog", "Careers", "Contact", "Privacy"],
  },
];

function SiteFooter() {
  return (
    <footer className="border-t border-zinc-200/70 bg-white">
      <div className="mx-auto max-w-5xl px-6 py-14">
        <div className="grid gap-10 sm:grid-cols-2 lg:grid-cols-[1.4fr_repeat(4,1fr)]">
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
                  <a
                    key={l}
                    href="#"
                    className="text-[13px] text-zinc-500 transition-colors hover:text-zinc-900"
                  >
                    {l}
                  </a>
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
