import React, { Suspense, use } from "react";
import { Link } from "@pylonsync/react";
import type { DbReader } from "@pylonsync/functions";

interface Marker {
  id: string;
  note: string;
}

interface PageProps {
  // Read-only server data handle — reaches the Pylon backend during SSR,
  // honoring the same policies as a query function's ctx.db. Writes rejected.
  serverData: DbReader;
}

// Renders inside <Suspense>. `use()` suspends on the serverData promise; the
// shell + fallback flush first, then this streams in once the rows arrive.
// On the client it resolves from the SSR'd data — no refetch, no mismatch.
function MarkerList({ serverData }: PageProps) {
  const markers = use(serverData.list("Marker")) as unknown as Marker[];
  if (markers.length === 0) {
    return <li className="text-zinc-500">(no markers yet)</li>;
  }
  return (
    <>
      {markers.map((m) => (
        <li key={m.id} className="font-mono text-sm">
          {m.note}
        </li>
      ))}
    </>
  );
}

export default function DataPage({ serverData }: PageProps) {
  return (
    <div className="space-y-6">
      <h1 className="text-3xl font-semibold tracking-tight">
        Server data via <code>use()</code> + Suspense
      </h1>
      <p className="text-zinc-600">
        The list below is fetched on the server during SSR through{" "}
        <code>serverData.list(&quot;Marker&quot;)</code>, streamed in after the
        shell, and hydrated from the same data — no client refetch.
      </p>

      <ul className="rounded-2xl border border-zinc-200 bg-white p-6 space-y-1">
        <Suspense
          fallback={<li className="text-zinc-400">Loading markers…</li>}
        >
          <MarkerList serverData={serverData} />
        </Suspense>
      </ul>

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
