import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";

// `app/not-found.tsx` → rendered at HTTP 404 for any unmatched URL (and when a
// page calls `response.notFound()` — e.g. a listing slug that doesn't resolve).
// Hydrated, so the link is a client nav.
export default function NotFound(_props: NotFoundProps) {
  return (
    <div className="mx-auto flex min-h-[60vh] max-w-3xl flex-col items-center justify-center px-6 text-center">
      <h1 className="text-3xl font-semibold tracking-tight">404</h1>
      <p className="mt-2 text-muted-foreground">We couldn&apos;t find that listing.</p>
      <Link
        href="/"
        className="mt-6 inline-flex min-h-11 items-center rounded-lg bg-foreground px-5 text-sm font-medium text-background transition-[opacity,scale] duration-150 hover:opacity-90 active:scale-[0.96]"
      >
        Back to browse
      </Link>
    </div>
  );
}
