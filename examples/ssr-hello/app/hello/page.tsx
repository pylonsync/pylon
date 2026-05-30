import React from "react";

interface PageProps {
  url: string;
  params: Record<string, string>;
  searchParams: Record<string, string>;
}

export default function HelloPage({ url, searchParams }: PageProps) {
  const name = searchParams.name ?? "world";
  return (
    <html>
      <head>
        <title>Hello — Pylon SSR</title>
      </head>
      <body>
        <h1>Hello, {name}!</h1>
        <p>Rendered server-side by Pylon at <code>{url}</code>.</p>
      </body>
    </html>
  );
}
