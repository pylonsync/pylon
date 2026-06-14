import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { Button } from "@/components/ui/button";

// A layout receives the page props plus `children`. `auth.user_id` is null for
// anonymous visitors and the signed-in user's id otherwise — resolved
// server-side from the session cookie before any HTML is sent, so the nav
// renders the right links on the first byte (no flash, no client fetch).
interface LayoutProps {
  children: React.ReactNode;
  url: string;
  auth: PageAuth;
}

// The root layout wraps every page: a marketing nav up top, a footer below.
// Rebrand "Acme" to your product.
export default function RootLayout({ children, auth }: LayoutProps) {
  const signedIn = Boolean(auth?.user_id);
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>Acme</title>
        {/* Tailwind is compiled by Pylon from app/globals.css and the
            stylesheet link is injected here automatically. */}
      </head>
      <body className="flex min-h-screen flex-col bg-background text-foreground antialiased">
        <header className="sticky top-0 z-20 border-b bg-background/70 backdrop-blur">
          <div className="mx-auto flex max-w-5xl items-center justify-between px-4 py-3">
            <Link href="/" className="flex items-center gap-2">
              <span className="flex size-7 items-center justify-center rounded-md bg-primary text-sm font-bold text-primary-foreground">
                A
              </span>
              <span className="text-sm font-semibold tracking-tight">Acme</span>
            </Link>
            <nav className="flex items-center gap-2 text-sm">
              {signedIn ? (
                <Button asChild size="sm">
                  <Link href="/dashboard">Dashboard</Link>
                </Button>
              ) : (
                <>
                  <Button asChild size="sm" variant="ghost">
                    <Link href="/login">Sign in</Link>
                  </Button>
                  <Button asChild size="sm">
                    <Link href="/signup">Get started</Link>
                  </Button>
                </>
              )}
            </nav>
          </div>
        </header>

        <main className="mx-auto w-full max-w-5xl flex-1 px-4 py-10">
          {children}
        </main>

        <footer className="border-t">
          <div className="mx-auto flex max-w-5xl flex-col items-center justify-between gap-2 px-4 py-6 text-xs text-muted-foreground sm:flex-row">
            <span>© Acme, Inc.</span>
            <span>
              Built with{" "}
              <a
                href="https://pylonsync.com"
                className="font-medium text-foreground hover:underline"
              >
                Pylon
              </a>{" "}
              · one binary, one port
            </span>
          </div>
        </footer>
      </body>
    </html>
  );
}
