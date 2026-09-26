import React from "react";

// Document shell. Fonts (Geist, Geist Mono) are declared in app.ts and
// self-hosted by the build; Tailwind is compiled from app/globals.css. Both
// are injected into <head> by the runtime.
export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />
        <meta name="theme-color" content="#f7f7f9" />
      </head>
      <body>{children}</body>
    </html>
  );
}
