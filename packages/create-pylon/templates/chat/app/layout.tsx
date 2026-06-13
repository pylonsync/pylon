import React from "react";

interface LayoutProps {
  children: React.ReactNode;
}

// Full-height layout so the chat can fill the screen with a sticky composer
// at the bottom. The page renders server-side first, then hydrates into the
// live room.
export default function RootLayout({ children }: LayoutProps) {
  return (
    <html lang="en" className="h-full">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>__APP_NAME__</title>
      </head>
      <body className="h-full bg-background text-foreground antialiased">
        <div className="mx-auto flex h-full max-w-lg flex-col px-4">
          {children}
        </div>
      </body>
    </html>
  );
}
