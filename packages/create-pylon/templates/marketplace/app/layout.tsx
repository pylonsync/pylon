import React from "react";
import { Link } from "@pylonsync/react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { AuthNav } from "../client/AuthNav";
import { ThemeToggle } from "../client/ThemeToggle";

// Root layout. Server-rendered shell; Pylon's SSR head adapter injects the
// compiled Tailwind <link> from app/globals.css automatically.
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
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
          content="#f8f6f2"
          media="(prefers-color-scheme: light)"
        />
        <meta
          name="theme-color"
          content="#181817"
          media="(prefers-color-scheme: dark)"
        />
        <link rel="preconnect" href="https://images.unsplash.com" />
      </head>
      <body className="min-h-screen bg-background text-foreground antialiased">
        <a
          href="#main-content"
          className="fixed left-4 top-4 z-50 -translate-y-20 rounded-lg bg-foreground px-4 py-2 text-sm font-medium text-background shadow-lg transition-transform focus:translate-y-0"
        >
          Skip to content
        </a>
        <header className="sticky top-0 z-20 h-16 border-b bg-background/85 backdrop-blur-xl">
          <div className="mx-auto flex h-full max-w-6xl items-center justify-between gap-3 px-5">
            <Link
              href="/"
              translate="no"
              className="flex min-h-11 items-center gap-2.5 text-sm font-semibold tracking-[-0.01em]"
            >
              <span className="grid size-8 place-items-center rounded-lg bg-foreground text-sm font-semibold text-background shadow-sm">
                R
              </span>
              Reprise
            </Link>
            <nav
              aria-label="Primary navigation"
              className="flex items-center gap-0.5 text-sm sm:gap-1"
            >
              <Button asChild variant="ghost" size="sm" className="hidden sm:inline-flex">
                <Link href="/">Browse</Link>
              </Button>
              <Button asChild variant="ghost" size="sm">
                <Link href="/sell">Sell</Link>
              </Button>
              <Button asChild variant="ghost" size="sm" className="hidden md:inline-flex">
                <Link href="/me">Dashboard</Link>
              </Button>
              <ThemeToggle />
              <AuthNav />
            </nav>
          </div>
        </header>
        <main
          id="main-content"
          className="mx-auto min-h-[calc(100dvh-8rem)] max-w-6xl px-5 py-6 sm:py-8"
        >
          {children}
        </main>
        <Separator />
        <footer className="px-5 py-7 text-center text-xs text-muted-foreground">
          Reprise, built with Pylon. Server-rendered listings and realtime offers
          from one binary.
        </footer>
      </body>
    </html>
  );
}
