import React from "react";

interface LayoutProps {
  children: React.ReactNode;
}

export default function RootLayout({ children }: LayoutProps) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>Pylon Sim — Co-op City Builder</title>
        <meta
          name="description"
          content="Build a city together. A multiplayer SimCity-style builder on three.js + Pylon SSR — one binary, one port, every tab a co-mayor."
        />
      </head>
      <body className="h-full overflow-hidden bg-[#0a0e14] text-zinc-100 antialiased">
        {children}
      </body>
    </html>
  );
}
