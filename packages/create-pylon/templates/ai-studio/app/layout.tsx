import React from "react";
import { Link, type PageAuth } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";

// App shell: a slim top bar over the studio. `auth.user_id` is resolved
// server-side from the session cookie before any HTML is sent, so the bar shows
// the account / "Sign in" with no flash. The header is h-14; the studio page
// fills the rest of the viewport (min-h-[calc(100vh-3.5rem)]).
interface LayoutProps {
  children: React.ReactNode;
  url: string;
  auth: PageAuth;
}

export default function RootLayout({ children, url, auth }: LayoutProps) {
  const signedIn = Boolean(auth?.user_id);
  const { brand, colors } = siteConfig;

  // The auth screen brings its own chrome → render it bare.
  const path = (url ?? "").split("?")[0];
  const isBare = path === "/login" || path.startsWith("/login/");

  return (
    <html
      lang="en"
      style={
        {
          "--brand": colors.brand,
          "--brand-soft": colors.brandSoft,
          "--paper": colors.paper,
        } as React.CSSProperties
      }
    >
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        {/* Inter is declared in app.ts (fonts: [...]) and self-hosted by the
            build — the runtime injects @font-face + <link rel=preload> + a
            size-adjusted fallback here automatically. No third-party request,
            no layout shift; change the family in app.ts. */}
      </head>
      <body className="bg-background text-foreground antialiased">
        {isBare ? (
          children
        ) : (
          <>
            <header className="flex h-14 items-center justify-between border-b border-zinc-200 bg-white px-4">
              <Link href="/" className="flex items-center gap-2">
                <span className="flex size-6 items-center justify-center rounded-[7px] bg-brand text-[13px] font-bold text-white">
                  {brand.letter}
                </span>
                <span className="text-[15px] font-semibold tracking-tight text-zinc-900">{brand.name}</span>
              </Link>
              {signedIn ? (
                <span className="text-[13px] text-zinc-400">Signed in</span>
              ) : (
                <Link
                  href="/login"
                  className="rounded-full border border-zinc-300 px-3.5 py-1.5 text-[13px] font-medium text-zinc-700 transition-colors hover:bg-zinc-50"
                >
                  Sign in
                </Link>
              )}
            </header>
            {children}
          </>
        )}
      </body>
    </html>
  );
}
