import React from "react";

interface LayoutProps {
  children: React.ReactNode;
}

/**
 * The document shell. The app chrome — sidebar, command palette, shortcuts —
 * lives in `app/workspace.tsx` instead, because it needs the synced data; this
 * layer only sets up the page.
 *
 * `class="dark"` makes dark the default theme. The light tokens in globals.css
 * are complete and tested, so removing the class flips the whole app; swap it
 * for a stored preference when you want a toggle.
 */
export default function RootLayout({ children }: LayoutProps) {
  return (
    <html lang="en" className="dark">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>__APP_NAME__</title>
        {/* Tailwind is compiled by Pylon from app/globals.css and the
            stylesheet link is injected here automatically. Inter is declared in
            app.ts (fonts: [...]) and self-hosted by the build — @font-face, the
            preload link, and a size-adjusted fallback are injected too. */}
      </head>
      <body className="h-full">{children}</body>
    </html>
  );
}
