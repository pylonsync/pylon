import React from "react";

interface LayoutProps {
  children: React.ReactNode;
}

// The root layout wraps every page: a header and a centered column. The page
// renders server-side first, then the feed hydrates into a live view.
export default function RootLayout({ children }: LayoutProps) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>__APP_NAME__</title>
      </head>
      <body className="min-h-screen bg-background text-foreground antialiased">
        <header className="sticky top-0 z-10 border-b bg-background/80 backdrop-blur">
          <div className="mx-auto max-w-lg px-4 py-3 text-sm font-semibold tracking-tight">
            __APP_NAME__
          </div>
        </header>
        <main className="mx-auto max-w-lg px-4 py-8">{children}</main>
      </body>
    </html>
  );
}
