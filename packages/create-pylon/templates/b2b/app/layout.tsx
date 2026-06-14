import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";

// A layout receives the page props plus `children`. `auth.user_id` is null
// for anonymous visitors and the signed-in user's id otherwise — resolved
// server-side from the session cookie, before any HTML is sent, so the nav
// renders the right links on the first byte (no flash, no client fetch). The
// `PageAuth` type is exported from @pylonsync/react so you never hand-roll it.
interface LayoutProps {
  children: React.ReactNode;
  url: string;
  auth: PageAuth;
}

// The root layout wraps every page.
export default function RootLayout({ children, auth }: LayoutProps) {
  const signedIn = Boolean(auth?.user_id);
  // Add `className="dark"` to this <html> to flip every shadcn token to its
  // dark value. The classes below use semantic tokens (bg-background,
  // text-foreground, …) so the whole UI re-themes from app/globals.css.
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>__APP_NAME__</title>
        {/* Tailwind is compiled by Pylon from app/globals.css and the
            stylesheet link is injected here automatically — nothing to
            wire up. */}
      </head>
      <body className="min-h-screen bg-background text-foreground antialiased">
        <header className="sticky top-0 z-10 border-b bg-background/80 backdrop-blur">
          <div className="mx-auto flex max-w-3xl items-center justify-between px-4 py-3">
            <Link
              href="/"
              className="text-sm font-semibold tracking-tight hover:text-muted-foreground"
            >
              __APP_NAME__
            </Link>
            <nav className="flex items-center gap-4 text-sm text-muted-foreground">
              <Link href="/" className="hover:text-foreground">
                Home
              </Link>
              {signedIn ? (
                <Link href="/dashboard" className="hover:text-foreground">
                  Dashboard
                </Link>
              ) : (
                <>
                  <Link href="/login" className="hover:text-foreground">
                    Sign in
                  </Link>
                  <Link
                    href="/signup"
                    className="rounded-md bg-primary px-3 py-1.5 font-medium text-primary-foreground hover:opacity-90"
                  >
                    Sign up
                  </Link>
                </>
              )}
            </nav>
          </div>
        </header>
        <main className="mx-auto max-w-3xl px-4 py-10">{children}</main>
        <footer className="border-t py-6 text-center text-xs text-muted-foreground">
          Rendered by Pylon · one server, one port
        </footer>
      </body>
    </html>
  );
}
