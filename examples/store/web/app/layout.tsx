import type { Metadata } from "next";
import "./globals.css";
import { PylonProvider } from "@/lib/pylon-client";
import { Header } from "@/components/header";

const SITE = process.env.NEXT_PUBLIC_SITE_URL ?? "http://localhost:5179";

export const metadata: Metadata = {
  metadataBase: new URL(SITE),
  title: {
    default: "Pylon Store — Faceted Search Showcase",
    template: "%s · Pylon Store",
  },
  description:
    "10,000-product demo storefront powered by Pylon's native full-text + faceted search. BM25 ranking, live facet counts, sub-millisecond filtering.",
  applicationName: "Pylon Store",
  keywords: [
    "pylon",
    "ecommerce",
    "faceted search",
    "full-text search",
    "BM25",
    "realtime",
  ],
  openGraph: {
    type: "website",
    siteName: "Pylon Store",
    title: "Pylon Store — Faceted Search Showcase",
    description:
      "10,000 products. Live facets. Sub-ms search. Built on Pylon.",
    url: SITE,
  },
  twitter: {
    card: "summary_large_image",
    title: "Pylon Store",
    description: "10,000 products. Live facets. Sub-ms search.",
  },
  robots: { index: true, follow: true },
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <link rel="preconnect" href="https://rsms.me/" />
        <link rel="stylesheet" href="https://rsms.me/inter/inter.css" />
      </head>
      <body className="flex min-h-screen flex-col">
        <PylonProvider>
          <Header />
          {children}
        </PylonProvider>
      </body>
    </html>
  );
}
