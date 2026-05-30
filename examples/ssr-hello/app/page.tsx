import React from "react";

interface PageProps {
  url: string;
}

export default function IndexPage({ url }: PageProps) {
  return (
    <html>
      <head>
        <title>Pylon SSR — index</title>
      </head>
      <body>
        <h1>Pylon SSR — Phase 1</h1>
        <p>
          You're at <code>{url}</code>. Try <a href="/hello">/hello</a> or{" "}
          <a href="/hello?name=eric">/hello?name=eric</a>.
        </p>
      </body>
    </html>
  );
}
