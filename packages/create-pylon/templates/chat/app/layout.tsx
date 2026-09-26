import React from "react";

interface LayoutProps {
  children: React.ReactNode;
}

// Full-height document shell. The chat fills the viewport; 404 and error
// pages center themselves inside it.
export default function RootLayout({ children }: LayoutProps) {
  return (
    <html lang="en" className="h-full">
      <head>
        <meta charSet="utf-8" />
        <meta
          name="viewport"
          content="width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content"
        />
        <meta name="theme-color" content="#fbfaf8" />
        <link
          rel="icon"
          href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E%3Crect width='32' height='32' rx='8' fill='%23d9623b'/%3E%3Cpath d='M9 11.5A2.5 2.5 0 0 1 11.5 9h9a2.5 2.5 0 0 1 2.5 2.5v6a2.5 2.5 0 0 1-2.5 2.5H15l-4 3.5V20h0a2 2 0 0 1-2-2z' fill='%23fff'/%3E%3C/svg%3E"
        />
        <title>__APP_NAME__</title>
      </head>
      <body className="h-full bg-background text-foreground antialiased">{children}</body>
    </html>
  );
}
