import React, { useState } from "react";

interface PageProps {
  url: string;
  searchParams: Record<string, string>;
}

export default function HelloPage({ url, searchParams }: PageProps) {
  const name = searchParams.name ?? "world";
  const [count, setCount] = useState(0);
  return (
    <>
      <h1>Hello, {name}!</h1>
      <p>Rendered server-side by Pylon at <code>{url}</code>.</p>
      <p>
        Interactive counter (proves hydration is live):{" "}
        <button
          type="button"
          onClick={() => setCount((c) => c + 1)}
          style={{ padding: "4px 10px" }}
        >
          clicked {count} time{count === 1 ? "" : "s"}
        </button>
      </p>
      <p style={{ color: "#888", fontSize: "12px" }}>
        If clicking the button updates the count without a page
        reload, the page hydrated.
      </p>
    </>
  );
}
