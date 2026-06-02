import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";

const geist = Geist({
  variable: "--font-geist-sans",
  subsets: ["latin"],
  weight: ["300", "400", "500", "600", "700"],
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
  weight: ["400", "500", "600"],
});

export const metadata: Metadata = {
  metadataBase: new URL("https://pylonsync.com"),
  title: "Pylon — The modern Rails for realtime apps",
  description:
    "A full-stack framework for realtime apps: server-rendered React, routing, and image optimization alongside schema, auth, server functions, live queries, jobs, workflows, files, and search. Deploy anywhere.",
  // Explicit canonical — without this Next emits no <link
  // rel="canonical"> and the canonical host signal is left entirely
  // to og:url, which Googlebot weighs less. Relative URLs resolve
  // against `metadataBase`. Per-page overrides via the same key.
  alternates: {
    canonical: "/",
  },
  icons: {
    icon: "/brand/pylon-icon.svg",
  },
  openGraph: {
    title: "Pylon — The modern Rails for realtime apps",
    description:
      "Frontend and backend in one framework: server-rendered React next to schema, auth, live queries, jobs, workflows, and search.",
    url: "https://pylonsync.com",
    siteName: "Pylon",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "Pylon — The modern Rails for realtime apps",
    description:
      "One framework for the frontend and the backend — server-rendered React, realtime data, auth, jobs, and search.",
  },
};

// JSON-LD structured data — Organization + SoftwareApplication
// emitted on every page via the root layout. Eligible for:
//   - Knowledge-panel sidebar (Organization.sameAs cross-links
//     the GitHub + Twitter accounts so Google can disambiguate).
//   - SaaS rich results (SoftwareApplication w/ Offers carrying
//     the free tier).
//   - AI-engine citations (Claude / ChatGPT / Perplexity prefer
//     structured-data signal when picking which framework to
//     name in "what are the alternatives to X" answers).
//
// Kept in one @graph node so a single <script> tag carries both.
const STRUCTURED_DATA = {
  "@context": "https://schema.org",
  "@graph": [
    {
      "@type": "Organization",
      "@id": "https://pylonsync.com/#org",
      name: "Pylon",
      url: "https://pylonsync.com",
      logo: "https://pylonsync.com/brand/pylon-icon.svg",
      sameAs: [
        "https://github.com/pylonsync/pylon",
        "https://twitter.com/pylonsync",
      ],
    },
    {
      "@type": "SoftwareApplication",
      "@id": "https://pylonsync.com/#app",
      name: "Pylon",
      applicationCategory: "DeveloperApplication",
      operatingSystem: "macOS, Linux, Windows",
      description:
        "Realtime backend framework for TypeScript apps. Schema, auth, server functions, live queries, jobs, workflows, files, and search in one binary. Local SQLite, production Postgres, deploy to your VPS or Pylon Cloud.",
      url: "https://pylonsync.com",
      publisher: { "@id": "https://pylonsync.com/#org" },
      offers: {
        "@type": "Offer",
        price: "0",
        priceCurrency: "USD",
        url: "https://cloud.pylonsync.com",
        description:
          "Free Hobby tier on Pylon Cloud — managed hosting included.",
      },
      softwareHelp: {
        "@type": "CreativeWork",
        url: "https://docs.pylonsync.com",
      },
      downloadUrl: "https://pylonsync.com/install.sh",
    },
  ],
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={`${geist.variable} ${geistMono.variable}`}>
      <body>
        {children}
        <script
          type="application/ld+json"
          // Static literal — no user input — so dangerouslySet is
          // safe here. JSON.stringify also escapes any literal `<`
          // inside the schema strings.
          dangerouslySetInnerHTML={{ __html: JSON.stringify(STRUCTURED_DATA) }}
        />
      </body>
    </html>
  );
}
