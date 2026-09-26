import React from "react";
import { siteConfig } from "@/lib/site.config";

// The document shell: <html>, <head>, <body>, and the brand colors from
// lib/site.config.ts as CSS variables. The paper and soft-brand colors apply
// to the light theme; globals.css sets their dark values. Fonts are declared in app.ts and
// injected into <head> by the runtime.
export default function RootLayout({ children }: { children: React.ReactNode }) {
  const { colors } = siteConfig;
  return (
    <html
      lang="en"
      style={
        {
          "--brand": colors.brand,
          "--brand-soft-light": colors.brandSoft,
          "--paper-light": colors.paper,
        } as React.CSSProperties
      }
    >
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />
        <meta name="theme-color" content="#fbfaf7" media="(prefers-color-scheme: light)" />
        <meta name="theme-color" content="#161513" media="(prefers-color-scheme: dark)" />
      </head>
      <body className="bg-bg text-ink antialiased">{children}</body>
    </html>
  );
}
