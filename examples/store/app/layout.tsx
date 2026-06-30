import React from "react";
import { StoreChrome } from "../client/StoreChrome";

// Root layout for the native-SSR Store. Each route under app/ is a real
// server-rendered page (catalog, /p/<slug>, /account, /checkout,
// /orders/<id>) — no hash router. The shared chrome (header, cart drawer,
// auth dialog, sync boot) mounts once here as a client island so it persists
// across client-side route transitions while each page's content SSRs.
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
        <StoreChrome />
        {children}
      </body>
    </html>
  );
}
