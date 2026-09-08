import React from "react";
import { Link } from "@pylonsync/react";
import { missingSiteConfig, site } from "@/lib/site";

interface LayoutProps {
  children: React.ReactNode;
}

/**
 * `(site)` is a route group: the parens segment never appears in a URL, so
 * `(site)/page.tsx` still serves `/`. This layout owns the chrome for every
 * public page. Keep it here rather than checking the pathname in a component,
 * so a page that should not have the header simply lives outside the group.
 */
export default function SiteLayout({ children }: LayoutProps) {
  return (
    <div className="flex min-h-screen flex-col">
      <SetupNotice />
      <Header />
      <main className="flex-1">{children}</main>
      <Footer />
    </div>
  );
}

/**
 * Renders while `lib/site.ts` still has blank fields. The legal pages below
 * are a starting point with your company details filled in by that file, so
 * an unfinished site should say so out loud rather than publish placeholder
 * terms. It disappears on its own once the fields are set.
 */
function SetupNotice() {
  const missing = missingSiteConfig();
  if (missing.length === 0) return null;
  return (
    <div className="border-b border-amber-300 bg-amber-50 px-4 py-3 text-center text-[13px] text-amber-900">
      Finish <code className="font-mono">lib/site.ts</code> before you publish
      this site. Still blank: {missing.join(", ")}. The privacy policy and
      terms need your details, and a lawyer should read them.
    </div>
  );
}

function Header() {
  return (
    <header className="sticky top-0 z-20 border-b border-line bg-surface/85 backdrop-blur">
      <div className="mx-auto flex h-16 max-w-5xl items-center justify-between px-6">
        <Link href="/" className="text-[15px] font-semibold tracking-tight">
          {site.name}
        </Link>
        <nav className="flex items-center gap-6 text-[13.5px] text-ink-muted">
          <a href="/#features" className="hidden transition-colors hover:text-ink sm:block">
            Features
          </a>
          <a href="/#faq" className="hidden transition-colors hover:text-ink sm:block">
            Questions
          </a>
          <Link href="/support" className="transition-colors hover:text-ink">
            Support
          </Link>
          <a
            href="/#get"
            className="rounded-full bg-brand px-4 py-2 text-[13px] font-medium text-brand-ink transition-opacity hover:opacity-90"
          >
            Get the app
          </a>
        </nav>
      </div>
    </header>
  );
}

function Footer() {
  const year = new Date().getFullYear();
  return (
    <footer className="border-t border-line bg-surface-soft">
      <div className="mx-auto flex max-w-5xl flex-col gap-4 px-6 py-10 text-[13px] text-ink-muted sm:flex-row sm:items-center sm:justify-between">
        <p>
          © {year} {site.company || site.name}
        </p>
        <nav className="flex flex-wrap items-center gap-5">
          <Link href="/privacy" className="transition-colors hover:text-ink">
            Privacy
          </Link>
          <Link href="/terms" className="transition-colors hover:text-ink">
            Terms
          </Link>
          <Link href="/support" className="transition-colors hover:text-ink">
            Support
          </Link>
          {site.supportEmail ? (
            <a
              href={`mailto:${site.supportEmail}`}
              className="transition-colors hover:text-ink"
            >
              {site.supportEmail}
            </a>
          ) : null}
        </nav>
      </div>
    </footer>
  );
}
