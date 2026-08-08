import React from "react";
import { siteConfig } from "@/lib/site.config";

// The root layout is only the document shell: <html>, <head>, <body>, and the
// theme variables. The app shell (top bar) lives in the `(chat)` route-group
// layout, so /login — outside the group — renders bare with its own chrome.
interface LayoutProps {
  children: React.ReactNode;
}

export default function RootLayout({ children }: LayoutProps) {
  const { colors } = siteConfig;

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
        {children}
      </body>
    </html>
  );
}
