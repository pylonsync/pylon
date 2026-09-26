import React from "react";

// Root layout for the native-SSR Trade example. Pylon's SSR head adapter
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
    <html lang="en" className="dark" suppressHydrationWarning>
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link rel="preconnect" href="https://fonts.gstatic.com" crossOrigin="" />
        <link
          rel="stylesheet"
          href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600&family=Geist+Mono:wght@400;500;600&display=swap"
        />
      </head>
      <body className="h-screen overflow-hidden bg-background text-foreground antialiased">
        {children}
      </body>
    </html>
  );
}
