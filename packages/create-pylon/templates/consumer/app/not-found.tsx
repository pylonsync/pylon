import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";

// `app/not-found.tsx` → rendered at HTTP 404 for any unmatched URL, and when a
// page calls `response.notFound()`.
export default function NotFound(_props: NotFoundProps) {
  return (
    <div className="mx-auto max-w-md px-6 py-24 text-center">
      <h1 className="text-xl font-semibold">This page is not available.</h1>
      <p className="mt-2 text-sm text-zinc-500">The link may be broken, or the page may have been removed.</p>
      <Link href="/" className="mt-6 inline-block text-sm font-semibold text-brand">
        Back to the feed
      </Link>
    </div>
  );
}
