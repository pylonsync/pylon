import React from "react";
import { Link } from "@pylonsync/react";

// Auth shape injected by the SSR runtime. `auth.user_id` is null for
// anonymous visitors. Wire a sign-in flow with @pylonsync/client when
// you're ready — for now this just shows the session state.
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

// The root layout wraps every page. It receives `url` and `auth` from the
// SSR runtime on every render — server-side, before the HTML is sent.
export default function RootLayout({ children, url, auth }: LayoutProps) {
  const signedIn = Boolean(auth?.user_id);
  return (
    <html lang="en" className="bg-zinc-50">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>__APP_NAME__</title>
        {/* Tailwind is compiled by Pylon from app/globals.css and the
            stylesheet link is injected here automatically — nothing to
            wire up. */}
      </head>
      <body className="min-h-screen text-zinc-900 antialiased">
        <header className="sticky top-0 z-10 border-b border-zinc-200 bg-white/80 backdrop-blur">
          <div className="mx-auto flex max-w-3xl items-center justify-between px-4 py-3">
            <Link
              href="/"
              className="text-sm font-semibold tracking-tight hover:text-zinc-600"
            >
              __APP_NAME__
            </Link>
            <nav className="flex items-center gap-4 text-sm text-zinc-600">
              <Link href="/" className="hover:text-zinc-900">
                Home
              </Link>
              <Link href="/counter" className="hover:text-zinc-900">
                Counter
              </Link>
              <span
                className={signedIn ? "text-emerald-600" : "text-zinc-400"}
                title={url}
              >
                {signedIn ? `· ${auth.user_id}` : "· anon"}
              </span>
            </nav>
          </div>
        </header>
        <main className="mx-auto max-w-3xl px-4 py-10">{children}</main>
        <footer className="border-t border-zinc-200 py-6 text-center text-xs text-zinc-500">
          Rendered by Pylon · one server, one port
        </footer>
      </body>
    </html>
  );
}
