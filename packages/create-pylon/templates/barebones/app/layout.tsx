import React from "react";

// A layout wraps every page. This one is a header and a centered column —
// the page renders server-side first (shell + copy in the HTML), then
// hydrates into the live list.
interface LayoutProps {
  children: React.ReactNode;
}

export default function RootLayout({ children }: LayoutProps) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>__APP_NAME__</title>
        {/* Tailwind is compiled by Pylon from app/globals.css and the
            stylesheet link is injected here automatically. */}
      </head>
      <body className="min-h-screen bg-background text-foreground antialiased">
        <main className="mx-auto flex min-h-screen max-w-lg flex-col px-4 py-12">
          {children}
        </main>
      </body>
    </html>
  );
}
