import React from "react";

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
      </head>
      <body style={{ margin: 0, background: "#101014", color: "#e8e8ec", fontFamily: "system-ui, sans-serif" }}>
        {children}
      </body>
    </html>
  );
}
