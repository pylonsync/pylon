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

export default function RootLayout({ children, url, auth }: LayoutProps) {
  const signedIn = Boolean(auth?.user_id);
  return (
    <html lang="en" className="bg-zinc-50">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>Pylon SSR</title>
        {/* Tailwind is compiled by the Pylon SSR bundler from
            app/globals.css and emitted as a hashed asset under
            /_pylon/build/. The link tag is injected by the SSR
            head adapter from the manifest — no work needed here. */}
      </head>
      <body className="min-h-screen text-zinc-900 antialiased">
        <header className="sticky top-0 z-10 border-b border-zinc-200 bg-white/80 backdrop-blur">
          <div className="mx-auto flex max-w-3xl items-center justify-between px-4 py-3">
            <Link
              href="/"
              className="text-sm font-semibold tracking-tight hover:text-zinc-600"
            >
              Pylon SSR
            </Link>
            <nav className="flex items-center gap-4 text-sm text-zinc-600">
              <Link href="/" className="hover:text-zinc-900">
                Home
              </Link>
              <Link href="/hello" className="hover:text-zinc-900">
                Hello
              </Link>
              <Link href="/gallery" className="hover:text-zinc-900">
                Gallery
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
          Rendered by Pylon · Phase 2 (Link + Image)
        </footer>
      </body>
    </html>
  );
}
