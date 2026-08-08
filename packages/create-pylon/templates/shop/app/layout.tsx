import React from "react";
import { siteConfig } from "@/lib/site.config";
import { SectionScroller } from "@/components/section-scroller";

interface LayoutProps {
  children: React.ReactNode;
}

// The root layout is only the document shell: <html>, <head>, <body>, and the
// site-wide theme variables. Section chrome lives in route-group layouts —
// `(marketing)/layout.tsx` adds the nav + footer for the storefront pages.
// /login and /dashboard sit outside the group and render bare here: the auth
// screen and the dashboard bring their own chrome.
export default function RootLayout({ children }: LayoutProps) {
  const { colors } = siteConfig;
  return (
    <html
      lang="en"
      // Marketing theme colors come from the single site config. Set as inline
      // CSS vars on <html> so they override globals.css and the whole page
      // re-themes from one place — no CSS edit needed.
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
        {/* No <title> here — each page's exported `metadata` sets it. */}
        {/* Inter is declared in app.ts (fonts: [...]) and self-hosted by the
            build — the runtime injects @font-face + <link rel=preload> + a
            size-adjusted fallback here automatically. No third-party request,
            no layout shift; change the family in app.ts. */}
        {/* Tailwind is compiled by Pylon from app/globals.css and injected here. */}
      </head>
      <body className="flex min-h-screen flex-col bg-background text-foreground antialiased">
        <SectionScroller />
        {children}
      </body>
    </html>
  );
}
