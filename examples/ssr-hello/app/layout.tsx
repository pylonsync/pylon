import React from "react";

interface AuthShape {
  user_id: string | null;
  is_admin: boolean;
  tenant_id: string | null;
  roles: string[];
}

interface LayoutProps {
  children: React.ReactNode;
  url: string;
  auth: AuthShape;
}

export default function RootLayout({ children, url, auth }: LayoutProps) {
  const signedIn = Boolean(auth?.user_id);
  return (
    <html>
      <head>
        <meta charSet="utf-8" />
        <title>Pylon SSR — root layout</title>
      </head>
      <body>
        <header
          style={{
            borderBottom: "1px solid #ddd",
            padding: "8px 16px",
            display: "flex",
            justifyContent: "space-between",
          }}
        >
          <span>
            <strong>Pylon SSR</strong>{" "}
            <span style={{ color: "#888" }}>· {url}</span>
          </span>
          <span style={{ color: signedIn ? "#080" : "#888" }}>
            {signedIn ? `signed in (${auth.user_id})` : "anonymous"}
          </span>
        </header>
        <main style={{ padding: "16px" }}>{children}</main>
        <footer
          style={{ borderTop: "1px solid #ddd", padding: "8px 16px", color: "#888" }}
        >
          Rendered by Pylon · Phase 1.5b (auth context)
        </footer>
      </body>
    </html>
  );
}
