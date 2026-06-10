import React from "react";

// Root layout for the native-SSR ERP example. Pylon's SSR head adapter
// injects the compiled Tailwind <link> (from app/globals.css) into <head>
// automatically — no manual stylesheet wiring. The interactive, sync-engine
// driven UI mounts as a client island (see app/page.tsx), so this layout
// stays a thin server-rendered shell.
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <link rel="preconnect" href="https://rsms.me/" />
        <link rel="stylesheet" href="https://rsms.me/inter/inter.css" />
      </head>
      <body className="min-h-screen bg-background text-foreground antialiased">
        {children}
      </body>
    </html>
  );
}
