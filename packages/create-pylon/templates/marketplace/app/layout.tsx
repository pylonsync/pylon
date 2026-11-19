import React from "react";

// Root layout: the document shell for every route. The header and the footer
// live in `app/(market)/layout.tsx`. Server-rendered; Pylon's SSR head adapter injects the
// compiled Tailwind <link> from app/globals.css and the fonts from app.ts.
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <script
          dangerouslySetInnerHTML={{
            __html:
              "try{var t=localStorage.getItem('reprise:theme');document.documentElement.dataset.theme=t==='light'||t==='dark'?t:matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'}catch(e){}",
          }}
        />
        <meta
          name="theme-color"
          content="#f5f2ed"
          media="(prefers-color-scheme: light)"
        />
        <meta
          name="theme-color"
          content="#191714"
          media="(prefers-color-scheme: dark)"
        />
      </head>
      <body className="min-h-screen bg-background text-foreground antialiased">
        {children}
      </body>
    </html>
  );
}
