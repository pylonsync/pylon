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
        <link rel="preconnect" href="https://rsms.me/" />
        <link rel="stylesheet" href="https://rsms.me/inter/inter.css" />
      </head>
      <body className="min-h-screen bg-background text-foreground antialiased">
        <header className="sticky top-0 z-20 h-14 border-b bg-background/90 backdrop-blur">
          <div className="mx-auto flex h-full max-w-5xl items-center justify-between gap-4 px-5">
            <Link href="/" className="flex items-center gap-2 font-medium text-sm">
              <span className="grid size-7 place-items-center rounded-md bg-foreground text-sm font-semibold text-background">
                M
              </span>
              Pylon Market
            </Link>
            <nav className="flex items-center gap-1 text-sm">
              <Link
                href="/"
                className="rounded-md px-3 py-1.5 text-muted-foreground transition hover:bg-muted hover:text-foreground"
              >
                Browse
              </Link>
              <Link
                href="/sell"
                className="rounded-md px-3 py-1.5 text-muted-foreground transition hover:bg-muted hover:text-foreground"
              >
                Sell
              </Link>
              <Link
                href="/me"
                className="rounded-md px-3 py-1.5 text-muted-foreground transition hover:bg-muted hover:text-foreground"
              >
                My Market
              </Link>
              <AuthNav />
            </nav>
          </div>
        </header>
        <main className="mx-auto max-w-5xl px-5 py-6">{children}</main>
        <footer className="border-t py-6 text-center text-xs text-muted-foreground">
          Pylon Market · server-rendered listings + realtime offers from one
          binary
        </footer>
      </body>
    </html>
  );
}
