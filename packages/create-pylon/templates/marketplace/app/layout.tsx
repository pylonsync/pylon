import React from "react";
import { Link } from "@pylonsync/react";
import { AuthNav } from "../client/AuthNav";

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
      </head>
      <body className="min-h-screen bg-background text-foreground antialiased">
        <header className="sticky top-0 z-20 h-16 border-b bg-background/85 backdrop-blur-xl">
          <div className="mx-auto flex h-full max-w-6xl items-center justify-between gap-3 px-5">
            <Link
              href="/"
              className="flex min-h-11 items-center gap-2.5 text-sm font-semibold tracking-[-0.01em]"
            >
              <span className="grid size-8 place-items-center rounded-lg bg-foreground text-sm font-semibold text-background shadow-sm">
                R
              </span>
              Reprise
            </Link>
            <nav className="flex items-center gap-0.5 text-sm sm:gap-1">
              <Link
                href="/"
                className="hidden min-h-10 items-center rounded-lg px-3 text-muted-foreground transition-[background-color,color,scale] duration-150 hover:bg-muted hover:text-foreground active:scale-[0.96] sm:inline-flex"
              >
                Browse
              </Link>
              <Link
                href="/sell"
                className="inline-flex min-h-10 items-center rounded-lg px-3 text-muted-foreground transition-[background-color,color,scale] duration-150 hover:bg-muted hover:text-foreground active:scale-[0.96]"
              >
                Sell
              </Link>
              <Link
                href="/me"
                className="hidden min-h-10 items-center rounded-lg px-3 text-muted-foreground transition-[background-color,color,scale] duration-150 hover:bg-muted hover:text-foreground active:scale-[0.96] md:inline-flex"
              >
                Dashboard
              </Link>
              <AuthNav />
            </nav>
          </div>
        </header>
        <main className="mx-auto min-h-[calc(100dvh-8rem)] max-w-6xl px-5 py-6 sm:py-8">
          {children}
        </main>
        <footer className="border-t px-5 py-7 text-center text-xs text-muted-foreground">
          Reprise, built with Pylon. Server-rendered listings and realtime offers
          from one binary.
        </footer>
      </body>
    </html>
  );
}
