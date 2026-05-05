import type { Metadata } from "next";
import "./globals.css";
import { PylonProvider } from "@/lib/pylon-client";

export const metadata: Metadata = {
  title: "Pylon ERP",
  description: "Pylon ERP — Pylon example app",
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
      <body className="min-h-screen bg-background text-foreground antialiased">
        <PylonProvider>{children}</PylonProvider>
      </body>
    </html>
  );
}
