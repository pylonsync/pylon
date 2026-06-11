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
        <title>Pylon World3D — Island</title>
        <meta
          name="description"
          content="Multiplayer procedural-island FPS demo. three.js + Pylon SSR + realtime sync, one binary, one port."
        />
      </head>
      <body className="h-full overflow-hidden bg-[#04070c] text-zinc-100 antialiased">
        {children}
      </body>
    </html>
  );
}
