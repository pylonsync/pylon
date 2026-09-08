import React from "react";

interface LayoutProps {
  children: React.ReactNode;
}

/**
 * The document shell. Chrome lives in the section layout
 * (`app/(site)/layout.tsx`), which is what wraps the pages with the header
 * and footer. Tailwind's compiled stylesheet is injected into <head> by the
 * runtime.
 *
 * No <title> here on purpose: each page exports its own `metadata`, and a
 * title in the layout would render first and win.
 */
export default function RootLayout({ children }: LayoutProps) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <meta name="color-scheme" content="light dark" />
      </head>
      <body className="min-h-screen bg-surface text-ink">{children}</body>
    </html>
  );
}
