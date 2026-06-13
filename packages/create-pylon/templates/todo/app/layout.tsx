import React from "react";

// A layout wraps every page. This one is intentionally minimal — a header
// and a centered column. The page below it is server-rendered first (so the
// shell and copy are in the HTML), then hydrates into the live todo UI.
interface LayoutProps {
  children: React.ReactNode;
}

export default function RootLayout({ children }: LayoutProps) {
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
        <main className="mx-auto flex min-h-screen max-w-xl flex-col px-4 py-12">
          {children}
        </main>
      </body>
    </html>
  );
}
