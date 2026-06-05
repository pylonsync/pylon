import React from "react";
import { Button } from "@/components/ui/button";

interface PageProps {
  url: string;
  searchParams: Record<string, string>;
}

// `app/counter/page.tsx` → `/counter`. This page is server-rendered AND
// interactive: the HTML arrives with the initial count already in it (try
// /counter?start=10), then the per-route chunk hydrates and useState takes
// over. No client/server split to manage — it's one component. The buttons
// are shadcn/ui `Button`s, hydrated in place.
export default function CounterPage({ searchParams }: PageProps) {
  const start = Number(searchParams.start ?? "0") || 0;
  const [count, setCount] = React.useState(start);
  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-semibold tracking-tight">Counter</h1>
      <p className="text-muted-foreground">
        Rendered on the server, hydrated in the browser. The buttons work
        because the page's JS chunk hydrated this exact markup.
      </p>
      <div className="flex items-center gap-4">
        <Button
          variant="outline"
          size="icon"
          onClick={() => setCount((c) => c - 1)}
          aria-label="Decrement"
        >
          −
        </Button>
        <span className="min-w-12 text-center text-2xl font-semibold tabular-nums">
          {count}
        </span>
        <Button
          variant="outline"
          size="icon"
          onClick={() => setCount((c) => c + 1)}
          aria-label="Increment"
        >
          +
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Initial value comes from <code>?start=</code> — search params flow
        through SSR. Try{" "}
        <a
          href="/counter?start=10"
          className="text-primary underline-offset-4 hover:underline"
        >
          /counter?start=10
        </a>
        .
      </p>
    </div>
  );
}
