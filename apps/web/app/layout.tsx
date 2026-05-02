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
    "An integrated framework for realtime apps: schema, auth, server functions, live queries, jobs, workflows, files, search, and deploy-anywhere optionality.",
  icons: {
    icon: "/brand/pylon-icon.svg",
  },
  openGraph: {
    title: "Pylon — The modern Rails for realtime apps",
    description:
      "Schema, auth, server functions, live queries, jobs, workflows, files, and search in one framework.",
    url: "https://pylonsync.com",
    siteName: "Pylon",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "Pylon — The modern Rails for realtime apps",
    description:
      "One integrated framework for the backend, realtime UI, jobs, workflows, files, and search.",
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
