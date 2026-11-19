import React from "react";

// The layout for the product pages: a centered column. The page renders on
// the server first, then hydrates into the live todo list. `(app)` is a route
// group, so it does not show in the URL.
interface AppLayoutProps {
  children: React.ReactNode;
}

export default function AppLayout({ children }: AppLayoutProps) {
  return (
    <main className="mx-auto flex min-h-screen max-w-xl flex-col px-4 py-12">
      {children}
    </main>
  );
}
