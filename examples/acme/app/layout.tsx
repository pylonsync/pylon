import React from "react";
import { Link } from "@pylonsync/react";

interface AuthShape {
  user_id: string | null;
  is_admin: boolean;
  tenant_id: string | null;
  roles: string[];
}

interface LayoutProps {
  children: React.ReactNode;
  url: string;
  auth: AuthShape;
}

export default function RootLayout({ children, url }: LayoutProps) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>Acme — The intelligent operating system for modern teams</title>
        <meta
          name="description"
          content="Acme replaces the patchwork of tools your team is stitching together. One workspace, one keyboard shortcut, one source of truth."
        />
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link
          rel="preconnect"
          href="https://fonts.gstatic.com"
          crossOrigin="anonymous"
        />
        <link
          href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700;800&display=swap"
          rel="stylesheet"
        />
      </head>
      <body className="min-h-screen antialiased font-sans">
        <Nav url={url} />
        {children}
        <Footer />
      </body>
    </html>
  );
}

function Nav({ url }: { url: string }) {
  const links = [
    { href: "/pricing", label: "Pricing" },
    { href: "/about", label: "About" },
    { href: "/blog", label: "Blog" },
  ];
  return (
    <header className="sticky top-0 z-30 border-b border-[var(--color-line)] bg-[var(--color-cream)]/85 backdrop-blur">
      <div className="mx-auto flex h-16 max-w-6xl items-center justify-between px-6">
        <Link
          href="/"
          className="flex items-center gap-2 text-[var(--color-ink)]"
        >
          <BrandMark />
          <span className="text-base font-semibold tracking-tight">Acme</span>
        </Link>
        <nav className="hidden items-center gap-8 text-sm text-[var(--color-stone)] md:flex">
          {links.map((l) => (
            <Link
              key={l.href}
              href={l.href}
              className={
                "transition hover:text-[var(--color-ink)] " +
                (url.startsWith(l.href) ? "text-[var(--color-ink)]" : "")
              }
            >
              {l.label}
            </Link>
          ))}
        </nav>
        <div className="flex items-center gap-3">
          <Link
            href="/sign-in"
            className="hidden text-sm text-[var(--color-stone)] transition hover:text-[var(--color-ink)] md:inline"
          >
            Sign in
          </Link>
          <Link
            href="/sign-up"
            className="rounded-full bg-[var(--color-ink)] px-4 py-2 text-sm font-medium text-[var(--color-cream)] shadow-sm transition hover:bg-[var(--color-ink-soft)]"
          >
            Get started
          </Link>
        </div>
      </div>
    </header>
  );
}

function BrandMark() {
  return (
    <span
      aria-hidden
      className="inline-flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--color-brand)] text-[var(--color-cream)]"
    >
      <span className="text-base font-bold">A</span>
    </span>
  );
}

function Footer() {
  const cols = [
    {
      heading: "Product",
      links: [
        { href: "/pricing", label: "Pricing" },
        { href: "/changelog", label: "Changelog" },
        { href: "/security", label: "Security" },
        { href: "/integrations", label: "Integrations" },
      ],
    },
    {
      heading: "Company",
      links: [
        { href: "/about", label: "About" },
        { href: "/blog", label: "Blog" },
        { href: "/careers", label: "Careers" },
        { href: "/contact", label: "Contact" },
      ],
    },
    {
      heading: "Resources",
      links: [
        { href: "/docs", label: "Docs" },
        { href: "/community", label: "Community" },
        { href: "/customers", label: "Customers" },
        { href: "/status", label: "Status" },
      ],
    },
    {
      heading: "Legal",
      links: [
        { href: "/terms", label: "Terms" },
        { href: "/privacy", label: "Privacy" },
        { href: "/cookies", label: "Cookies" },
        { href: "/dpa", label: "DPA" },
      ],
    },
  ];
  return (
    <footer className="mt-32 border-t border-[var(--color-line)]">
      <div className="mx-auto max-w-6xl px-6 py-16">
        <div className="grid grid-cols-2 gap-10 md:grid-cols-5">
          <div className="col-span-2 md:col-span-1">
            <div className="flex items-center gap-2">
              <BrandMark />
              <span className="text-base font-semibold tracking-tight text-[var(--color-ink)]">
                Acme
              </span>
            </div>
            <p className="mt-4 max-w-xs text-sm text-[var(--color-stone)]">
              The intelligent operating system for modern teams. Built in
              Dallas.
            </p>
          </div>
          {cols.map((c) => (
            <div key={c.heading}>
              <h4 className="text-xs font-semibold uppercase tracking-wider text-[var(--color-ink)]">
                {c.heading}
              </h4>
              <ul className="mt-4 space-y-2.5 text-sm text-[var(--color-stone)]">
                {c.links.map((l) => (
                  <li key={l.href}>
                    <Link
                      href={l.href}
                      className="transition hover:text-[var(--color-ink)]"
                    >
                      {l.label}
                    </Link>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
        <div className="mt-16 flex flex-col items-start justify-between gap-4 border-t border-[var(--color-line)] pt-8 text-xs text-[var(--color-stone)] md:flex-row md:items-center">
          <span>© {new Date().getFullYear()} Acme, Inc. All rights reserved.</span>
          <span>Rendered by Pylon · one binary, one port.</span>
        </div>
      </div>
    </footer>
  );
}
