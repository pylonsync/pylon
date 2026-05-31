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
        <title>Acme</title>
        <meta
          name="description"
          content="Acme learns how you work, drafts the busywork, and hands you back the day."
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
    { href: "/about", label: "Enterprise" },
    { href: "/about", label: "Security" },
    { href: "/about", label: "About" },
  ];
  return (
    <header className="sticky top-0 z-30 bg-[var(--color-page)]/85 backdrop-blur">
      <div className="mx-auto grid h-16 max-w-7xl grid-cols-[1fr_auto_1fr] items-center gap-8 px-6">
        <Link
          href="/"
          className="flex items-center gap-2 justify-self-start text-[var(--color-ink)]"
        >
          <BrandMark />
          <span className="text-base font-semibold tracking-tight">Acme</span>
        </Link>
        <nav className="hidden items-center gap-8 text-sm text-[var(--color-ink-soft)] md:flex">
          {links.map((l, i) => (
            <Link
              key={i}
              href={l.href}
              className={
                "transition hover:opacity-60 " +
                (url === l.href ? "" : "")
              }
            >
              {l.label}
            </Link>
          ))}
        </nav>
        <div className="flex items-center gap-4 justify-self-end">
          <Link
            href="/sign-in"
            className="hidden text-sm text-[var(--color-ink-soft)] transition hover:opacity-60 md:inline"
          >
            Login
          </Link>
          <Link
            href="/sign-up"
            className="rounded-lg bg-[var(--color-ink)] px-3.5 py-1.5 text-sm font-medium text-white transition hover:bg-[var(--color-ink-soft)]"
          >
            Get Started
          </Link>
        </div>
      </div>
    </header>
  );
}

function BrandMark() {
  // Acme-style minimal logo mark — three stacked lines, soft blue.
  return (
    <svg
      aria-hidden
      viewBox="0 0 24 18"
      className="h-4 w-6 text-[var(--color-blue)]"
      fill="none"
    >
      <rect x="0" y="0" width="20" height="3" rx="1.5" fill="currentColor" />
      <rect x="2" y="6" width="18" height="3" rx="1.5" fill="currentColor" opacity="0.85" />
      <rect x="4" y="12" width="14" height="3" rx="1.5" fill="currentColor" opacity="0.65" />
    </svg>
  );
}

function Footer() {
  const cols = [
    {
      heading: "Product",
      links: [
        { href: "/pricing", label: "Pricing" },
        { href: "/", label: "Changelog" },
        { href: "/", label: "Security" },
        { href: "/", label: "Integrations" },
      ],
    },
    {
      heading: "Company",
      links: [
        { href: "/about", label: "About" },
        { href: "/blog", label: "Blog" },
        { href: "/", label: "Careers" },
        { href: "/", label: "Contact" },
      ],
    },
    {
      heading: "Resources",
      links: [
        { href: "/", label: "Docs" },
        { href: "/", label: "Customers" },
        { href: "/", label: "Status" },
        { href: "/", label: "Brand" },
      ],
    },
    {
      heading: "Legal",
      links: [
        { href: "/", label: "Terms" },
        { href: "/", label: "Privacy" },
        { href: "/", label: "Cookies" },
        { href: "/", label: "DPA" },
      ],
    },
  ];
  return (
    <footer className="mt-32">
      <div className="mx-auto max-w-6xl px-6 py-16">
        <div className="grid grid-cols-2 gap-10 md:grid-cols-5">
          <div className="col-span-2 md:col-span-1">
            <div className="flex items-center gap-2">
              <BrandMark />
              <span className="text-base font-semibold tracking-tight text-[var(--color-ink)]">
                Acme
              </span>
            </div>
            <p className="mt-4 max-w-xs text-sm text-[var(--color-muted)]">
              The intelligent workspace.
            </p>
          </div>
          {cols.map((c) => (
            <div key={c.heading}>
              <h4 className="text-xs font-semibold uppercase tracking-wider text-[var(--color-ink)]">
                {c.heading}
              </h4>
              <ul className="mt-4 space-y-2.5 text-sm text-[var(--color-muted)]">
                {c.links.map((l, i) => (
                  <li key={i}>
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
        <div className="mt-16 flex flex-col items-start justify-between gap-2 pt-8 text-xs text-[var(--color-muted)] md:flex-row md:items-center">
          <span>© {new Date().getFullYear()} Acme, Inc.</span>
          <span>Made for teams that ship.</span>
        </div>
      </div>
    </footer>
  );
}
