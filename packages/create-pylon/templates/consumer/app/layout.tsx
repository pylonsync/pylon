import React from "react";

interface LayoutProps {
  children: React.ReactNode;
}

// The document shell. The app chrome (sidebar and mobile bars) lives in
// `(app)/layout.tsx`, so a page outside that group renders bare.
export default function RootLayout({ children }: LayoutProps) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>__APP_NAME__</title>
      </head>
      <body className="min-h-screen bg-background text-foreground antialiased">{children}</body>
    </html>
  );
}
