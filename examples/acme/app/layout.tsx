import React from "react";
import { Link } from "@pylonsync/react";
import type { Metadata } from "@pylonsync/react";

export const metadata: Metadata = {
  title: "Acme",
  description: "Acme keeps your team's projects, docs, and updates together in one place.",
};

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
      <body className="min-h-screen antialiased font-sans bg-background text-foreground">
        <Nav url={url} />
        {children}
        <Footer />
      </body>
    </html>
  );
}

const NAV_LINKS = [
  { href: "/pricing", label: "Pricing" },
  { href: "/about", label: "Enterprise" },
  { href: "/contact", label: "Security" },
  { href: "/about", label: "About" },
  { href: "/blog", label: "Blog" },
];

function Nav({ url }: { url: string }) {
  return (
    <header className="sticky top-0 z-30 h-14 border-b bg-background/85 backdrop-blur">
      <div className="mx-auto flex h-full max-w-7xl items-center justify-between gap-8 px-5">
        <Link
          href="/"
          className="flex items-center gap-2 text-foreground"
        >
          <BrandMark />
          <span className="text-sm font-medium tracking-tight">Acme</span>
        </Link>
        <nav className="hidden items-center gap-6 text-sm text-muted-foreground md:flex">
          {NAV_LINKS.map((l) => (
            <Link
              key={l.href + l.label}
              href={l.href}
              className={
                "transition hover:text-foreground " +
                (url === l.href ? "font-medium text-foreground" : "")
              }
            >
              {l.label}
            </Link>
          ))}
        </nav>
        <div className="flex items-center gap-3">
          <Link
            href="/contact"
            className="hidden text-sm text-muted-foreground transition hover:text-foreground md:inline"
          >
            Contact
          </Link>
          <Link
            href="/contact"
            className="rounded-lg bg-primary px-3.5 py-1.5 text-sm font-medium text-primary-foreground transition hover:opacity-90"
          >
            Get Started
          </Link>
        </div>
      </div>
    </header>
  );
}

function BrandMark() {
  return (
    <svg
      aria-hidden
      viewBox="0 0 24 24"
      className="h-5 w-5 text-[var(--color-blue)]"
      fill="none"
    >
      <rect x="8" y="8" width="13" height="13" rx="3.5" fill="currentColor" />
      <rect
        x="3"
        y="3"
        width="13"
        height="13"
        rx="3.5"
        stroke="currentColor"
        strokeWidth="2.25"
      />
    </svg>
  );
}

const FOOTER_COLS = [
  {
    heading: "Product",
    links: [
      { href: "/pricing", label: "Pricing" },
      { href: "/blog/changelog", label: "Changelog" },
      { href: "/contact", label: "Security" },
      { href: "/contact", label: "Integrations" },
    ],
  },
  {
    heading: "Company",
    links: [
      { href: "/about", label: "About" },
      { href: "/blog", label: "Blog" },
      { href: "/contact", label: "Careers" },
      { href: "/contact", label: "Contact" },
    ],
  },
  {
    heading: "Resources",
    links: [
      { href: "/contact", label: "Docs" },
      { href: "/contact", label: "Customers" },
      { href: "/contact", label: "Status" },
      { href: "/about", label: "Brand" },
    ],
  },
  {
    heading: "Legal",
    links: [
      { href: "/contact", label: "Terms" },
      { href: "/contact", label: "Privacy" },
      { href: "/contact", label: "Cookies" },
      { href: "/contact", label: "DPA" },
    ],
  },
];

function Footer() {
  return (
    <footer className="mt-32 border-t">
      <div className="mx-auto max-w-6xl px-6 py-16">
        <div className="grid grid-cols-2 gap-10 md:grid-cols-5">
          <div className="col-span-2 md:col-span-1">
            <div className="flex items-center gap-2">
              <BrandMark />
              <span className="text-sm font-medium tracking-tight text-foreground">
                Acme
              </span>
            </div>
            <p className="mt-4 max-w-xs text-sm text-muted-foreground">
              One home for your team's work.
            </p>
          </div>
          {FOOTER_COLS.map((c) => (
            <div key={c.heading}>
              <h4 className="text-xs font-semibold uppercase tracking-wider text-foreground">
                {c.heading}
              </h4>
              <ul className="mt-4 space-y-2.5 text-sm text-muted-foreground">
                {c.links.map((l) => (
                  <li key={l.label}>
                    <Link
                      href={l.href}
                      className="transition hover:text-foreground"
                    >
                      {l.label}
                    </Link>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
        <div className="mt-16 flex flex-col items-start justify-between gap-2 border-t pt-8 text-xs text-muted-foreground md:flex-row md:items-center">
          <span>© {new Date().getFullYear()} Acme, Inc.</span>
          <span>Made for teams that ship.</span>
        </div>
      </div>
    </footer>
  );
}
