import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";

// `(chat)/not-found.tsx` → rendered at HTTP 404 for any unmatched URL (and
// when a page calls `response.notFound()`). It lives inside the `(chat)` group
// on purpose: a group segment adds no URL prefix, so this is still the root
// 404 boundary, but it wraps in the app-shell layout — a missing URL gets the
// top bar instead of a bare page. Hydrated, so the link is a client nav.
export default function NotFound(_props: NotFoundProps) {
  return (
    <div className="mx-auto flex min-h-[60vh] max-w-3xl flex-col items-center justify-center px-6 text-center">
      <h1 className="text-3xl font-semibold tracking-tight">404</h1>
      <p className="mt-2 text-zinc-500">We couldn&apos;t find that page.</p>
      <Link
        href="/"
        className="mt-6 inline-flex h-10 items-center rounded-full bg-zinc-900 px-5 text-sm font-medium text-white transition-colors hover:bg-zinc-700"
      >
        Back home
      </Link>
    </div>
  );
}
