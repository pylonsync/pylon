import React from "react";
import { siteConfig } from "@/lib/site.config";

interface LayoutProps {
  children: React.ReactNode;
}

// The root layout is only the document shell: <html>, <head>, <body>, and the
// site-wide theme variables. Section chrome lives in route-group layouts —
// `(marketing)/layout.tsx` adds the nav + footer for every marketing page.
// Routes outside the group (/login, /signup, /dashboard) render bare here:
// the auth screens are full-screen and the dashboard brings its own sidebar
// shell.
export default function RootLayout({ children }: LayoutProps) {
  return (
    <html
      lang="en"
      // Marketing theme colors come from the single site config (lib/site.config.ts).
      // Set as inline CSS vars on <html> so they override globals.css defaults and
      // the whole site re-themes from one place — no CSS edit needed.
      style={
        {
          "--brand": siteConfig.colors.brand,
          "--brand-soft": siteConfig.colors.brandSoft,
          "--paper": siteConfig.colors.paper,
        } as React.CSSProperties
      }
    >
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        {/* No <title> here on purpose — each page's exported `metadata` /
            `generateMetadata` sets it. A hardcoded title in the layout would
            render first and win over the page's, so every tab would read
            "Acme". */}
        {/* Inter is declared in app.ts (`fonts: [font({ family: "Inter", … })]`)
            and self-hosted by the build — the runtime injects the @font-face,
            <link rel=preload>, and a size-adjusted fallback here automatically.
            No third-party Google Fonts request, no layout shift. Swap the family
            in app.ts to change it. Tailwind's compiled stylesheet is injected
            here too. */}
      </head>
      <body className="flex min-h-screen flex-col bg-background text-foreground antialiased">
        {children}
      </body>
    </html>
  );
}
