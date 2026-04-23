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
  title: "Pylon — The backend for real-time apps and games",
  description:
    "Declarative schema, live sync, TypeScript functions, and tick-based game shards — as a single Rust binary. Self-host or deploy to Cloudflare Workers, idle at $0.",
  icons: {
    icon: "/brand/pylon-icon.svg",
  },
  openGraph: {
    title: "Pylon — The backend for real-time apps and games",
    description:
      "Declarative schema, live sync, TypeScript functions, and tick-based game shards — as a single Rust binary.",
    url: "https://pylonsync.com",
    siteName: "Pylon",
    type: "website",
  },
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={`${geist.variable} ${geistMono.variable}`}>
      <body>{children}</body>
    </html>
  );
}
