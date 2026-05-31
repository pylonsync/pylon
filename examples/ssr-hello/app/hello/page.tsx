import React from "react";

interface PageProps {
  url: string;
  searchParams: Record<string, string>;
}

export default function HelloPage({ url, searchParams }: PageProps) {
  const name = searchParams.name ?? "world";
  return (
    <>
      <h1>Hello, {name}!</h1>
      <p>Rendered server-side by Pylon at <code>{url}</code>.</p>
    </>
  );
}
