import React from "react";

interface PageProps {
  url: string;
  searchParams: Record<string, string>;
}

// `app/counter/page.tsx` → `/counter`. This page is server-rendered AND
// interactive: the HTML arrives with the initial count already in it (try
// /counter?start=10), then the per-route chunk hydrates and useState takes
// over. No client/server split to manage — it's one component.
export default function CounterPage({ searchParams }: PageProps) {
  const start = Number(searchParams.start ?? "0") || 0;
  const [count, setCount] = React.useState(start);
  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-semibold tracking-tight">Counter</h1>
      <p className="text-zinc-600">
        Rendered on the server, hydrated in the browser. The buttons work
        because the page's JS chunk hydrated this exact markup.
      </p>
      <div className="flex items-center gap-4">
        <button
          onClick={() => setCount((c) => c - 1)}
          className="rounded-lg border border-zinc-300 px-4 py-2 text-lg hover:bg-zinc-100"
        >
          −
        </button>
        <span className="min-w-12 text-center text-2xl font-semibold tabular-nums">
          {count}
        </span>
        <button
          onClick={() => setCount((c) => c + 1)}
          className="rounded-lg border border-zinc-300 px-4 py-2 text-lg hover:bg-zinc-100"
        >
          +
        </button>
      </div>
      <p className="text-xs text-zinc-500">
        Initial value comes from <code>?start=</code> — search params flow
        through SSR. Try{" "}
        <a
          href="/counter?start=10"
          className="text-blue-600 underline-offset-4 hover:underline"
        >
          /counter?start=10
        </a>
        .
      </p>
    </div>
  );
}
