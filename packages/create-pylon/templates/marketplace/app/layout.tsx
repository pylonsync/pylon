import React from "react";
import { Link, type PageProps } from "@pylonsync/react";
import { Plus, Search } from "lucide-react";
import { AuthNav } from "../client/AuthNav";
import { ThemeToggle } from "../client/ThemeToggle";

// Root layout. Server-rendered shell; Pylon's SSR head adapter injects the
// compiled Tailwind <link> from app/globals.css and the fonts from app.ts.
export default function RootLayout({
  children,
  searchParams,
}: {
  children: React.ReactNode;
} & Partial<Pick<PageProps, "searchParams">>) {
  const query =
    typeof searchParams?.q === "string" ? searchParams.q.trim() : "";

  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <script
          dangerouslySetInnerHTML={{
            __html:
              "try{var t=localStorage.getItem('reprise:theme');document.documentElement.dataset.theme=t==='light'||t==='dark'?t:matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'}catch(e){}",
          }}
        />
        <meta
          name="theme-color"
          content="#f5f2ed"
          media="(prefers-color-scheme: light)"
        />
        <meta
          name="theme-color"
          content="#191714"
          media="(prefers-color-scheme: dark)"
        />
      </head>
      <body className="min-h-screen bg-background text-foreground antialiased">
        <a
          href="#main-content"
          className="fixed left-4 top-4 z-50 -translate-y-20 rounded-lg bg-foreground px-4 py-2 text-sm font-medium text-background transition-transform focus:translate-y-0"
        >
          Skip to content
        </a>
        <header className="sticky top-0 z-30 border-b border-border/70 bg-background/85 backdrop-blur-xl">
          <div className="mx-auto flex h-16 max-w-[1440px] items-center gap-3 px-4 sm:gap-6 sm:px-8">
            <Link
              href="/"
              translate="no"
              className="flex min-h-11 shrink-0 items-center gap-2"
              aria-label="Reprise home"
            >
              <span
                aria-hidden="true"
                className="grid size-8 place-items-center rounded-full bg-foreground pb-0.5 font-display text-[22px] leading-none text-background"
              >
                r
              </span>
              <span className="font-display text-[26px] leading-none tracking-[-0.01em]">
                Reprise
              </span>
            </Link>

            <form
              method="get"
              action="/"
              role="search"
              className="relative hidden max-w-xl flex-1 md:block"
            >
              <label htmlFor="header-search" className="sr-only">
                Search listings
              </label>
              <Search
                aria-hidden="true"
                strokeWidth={1.75}
                className="pointer-events-none absolute left-3.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
              />
              <input
                id="header-search"
                name="q"
                type="search"
                autoComplete="off"
                defaultValue={query}
                placeholder="Search chairs, cameras, sellers, cities"
                className="h-10 w-full rounded-full border-0 bg-muted pl-10 pr-4 text-sm outline-none ring-1 ring-transparent transition-[box-shadow,background-color] duration-200 placeholder:text-muted-foreground focus:bg-card focus:ring-border focus-visible:outline-none"
              />
            </form>

            <nav
              aria-label="Primary navigation"
              className="ml-auto flex items-center gap-1 text-sm"
            >
              <Link
                href="/me"
                className="hidden min-h-10 items-center rounded-full px-3 font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground sm:inline-flex"
              >
                Dashboard
              </Link>
              <ThemeToggle />
              <AuthNav />
              <Link
                href="/sell"
                className="ml-1 inline-flex min-h-10 items-center gap-1.5 rounded-full bg-primary pl-3 pr-4 text-sm font-medium text-primary-foreground transition-[background-color,scale] duration-150 hover:bg-primary/85 active:scale-[0.97]"
              >
                <Plus aria-hidden="true" className="size-4" strokeWidth={2} />
                Sell
              </Link>
            </nav>
          </div>
        </header>
        <main
          id="main-content"
          className="mx-auto min-h-[calc(100dvh-8rem)] max-w-[1440px] px-4 sm:px-8"
        >
          {children}
        </main>
        <footer className="mt-16 border-t border-border/70">
          <div className="mx-auto flex max-w-[1440px] flex-col gap-2 px-4 py-8 text-sm text-muted-foreground sm:flex-row sm:items-center sm:justify-between sm:px-8">
            <p>
              <span className="font-display text-lg text-foreground">Reprise</span>{" "}
              · Pre-owned goods from independent sellers
            </p>
            <p className="text-xs">
              Built with Pylon. Listings are server-rendered; offers and sold
              status sync live.
            </p>
          </div>
        </footer>
      </body>
    </html>
  );
}
