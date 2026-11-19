import React from "react";

// The root layout is the document shell for every route. Page chrome lives in
// a route group layout: `app/(app)/layout.tsx` for the product and
// `app/internal/layout.tsx` for the design reference.
interface LayoutProps {
  children: React.ReactNode;
}

export default function RootLayout({ children }: LayoutProps) {
  // Add `className="dark"` to this <html> to switch every design token to its
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
        {children}
      </body>
    </html>
  );
}
