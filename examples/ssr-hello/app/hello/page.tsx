import React, { useState } from "react";
import { Link } from "@pylonsync/react";

interface PageProps {
  url: string;
  searchParams: Record<string, string>;
}

export default function HelloPage({ url, searchParams }: PageProps) {
  const name = searchParams.name ?? "world";
  const [count, setCount] = useState(0);
  return (
    <div className="space-y-6">
      <h1 className="text-3xl font-semibold tracking-tight">
        Hello, <span className="text-blue-600">{name}</span>!
      </h1>
      <p className="text-zinc-600">
        Server-rendered at <code className="text-zinc-900">{url}</code>.
      </p>

      <div className="rounded-2xl border border-zinc-200 bg-white p-6">
        <p className="text-sm text-zinc-600">
          Counter (proves the client hydrated):
        </p>
        <div className="mt-3 flex items-center gap-3">
          <button
            type="button"
            onClick={() => setCount((c) => c + 1)}
            className="rounded-md bg-zinc-900 px-3 py-1.5 text-sm font-medium text-white shadow-sm transition hover:bg-zinc-700"
          >
            clicked {count} time{count === 1 ? "" : "s"}
          </button>
          <button
            type="button"
            onClick={() => setCount(0)}
            className="rounded-md border border-zinc-200 px-3 py-1.5 text-sm text-zinc-600 transition hover:bg-zinc-50"
          >
            reset
          </button>
        </div>
        <p className="mt-4 text-xs text-zinc-500">
          The count survives a client-side nav from <code>/</code> →{" "}
          <code>/hello</code> because the layout above stays mounted across
          routes.
        </p>
      </div>

      <p className="text-sm">
        <Link
          href="/"
          className="text-blue-600 underline-offset-4 hover:underline"
        >
          ← back home
        </Link>
      </p>
    </div>
  );
}
