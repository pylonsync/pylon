import React from "react";
import { Link } from "@pylonsync/react";

interface PageProps {
  url: string;
}

// `app/page.tsx` → `/`. Pages receive `{ url, auth, searchParams }` from
// the SSR runtime. This renders to HTML on the server; the per-route
// chunk hydrates it in the browser so interactive pages (see /counter)
// just work.
export default function IndexPage({ url }: PageProps) {
  return (
    <div className="space-y-8">
      <section>
        <h1 className="text-3xl font-semibold tracking-tight">__APP_NAME__</h1>
        <p className="mt-2 text-zinc-600">
          A full-stack Pylon app. Server-rendered React, file-based routes,
          a synced database, and a typed client — served from one binary on
          one port. No Next.js, no separate API server.
        </p>
      </section>

      <section className="rounded-2xl border border-zinc-200 bg-white p-6">
        <h2 className="text-lg font-semibold">Next steps</h2>
        <ul className="mt-3 space-y-2 text-sm text-zinc-700">
          <li>
            Add a route: drop a file at{" "}
            <code className="rounded bg-zinc-100 px-1">app/about/page.tsx</code>{" "}
            and visit <code className="rounded bg-zinc-100 px-1">/about</code>.
          </li>
          <li>
            Add data: edit{" "}
            <code className="rounded bg-zinc-100 px-1">app.ts</code> — every{" "}
            <code className="rounded bg-zinc-100 px-1">entity()</code> gets a
            REST + realtime API and a typed client automatically.
          </li>
          <li>
            See hydration in action on{" "}
            <Link
              href="/counter"
              className="text-blue-600 underline-offset-4 hover:underline"
            >
              /counter
            </Link>
            .
          </li>
        </ul>
      </section>

      <p className="text-xs text-zinc-500">
        You're at <code>{url}</code>. Edit{" "}
        <code>app/page.tsx</code> and save — the page reloads instantly.
      </p>
    </div>
  );
}
