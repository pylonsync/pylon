import React from "react";

interface PageProps {
  url: string;
}

export default function IndexPage({ url }: PageProps) {
  return (
    <>
      <h1>Pylon SSR — index page</h1>
      <p>
        You're at <code>{url}</code>. Try <a href="/hello">/hello</a> or{" "}
        <a href="/hello?name=eric">/hello?name=eric</a>.
      </p>
      <p style={{ color: "#888" }}>
        The header and footer above/below this section come from{" "}
        <code>app/layout.tsx</code> — the layout wraps every page.
      </p>
    </>
  );
}
