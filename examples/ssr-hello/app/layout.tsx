import React from "react";

interface LayoutProps {
  children: React.ReactNode;
  url: string;
}

export default function RootLayout({ children, url }: LayoutProps) {
  return (
    <html>
      <head>
        <meta charSet="utf-8" />
        <title>Pylon SSR — root layout</title>
      </head>
      <body>
        <header style={{ borderBottom: "1px solid #ddd", padding: "8px 16px" }}>
          <strong>Pylon SSR</strong> <span style={{ color: "#888" }}>· {url}</span>
        </header>
        <main style={{ padding: "16px" }}>{children}</main>
        <footer
          style={{ borderTop: "1px solid #ddd", padding: "8px 16px", color: "#888" }}
        >
          Rendered by Pylon · Phase 1.5 (layouts)
        </footer>
      </body>
    </html>
  );
}
