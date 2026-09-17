import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";

// `(marketing)/not-found.tsx` → rendered at HTTP 404 for any unmatched URL
// (and when a page calls `response.notFound()`). It lives inside the
// `(marketing)` group on purpose: a group segment adds no URL prefix, so this
// is still the root 404 boundary, but it wraps in the marketing layout, so a
// missing URL gets the site nav + footer instead of a bare page. Hydrated, so
// the link is a client nav.
export default function NotFound(_props: NotFoundProps) {
  return (
    <div className="mx-auto flex min-h-[60vh] w-full max-w-6xl flex-col justify-center px-6">
      <p className="font-mono-ui text-[13px] text-brand">404</p>
      <h1 className="font-display mt-4 text-3xl font-bold tracking-tight text-chalk">
        That page does not exist.
      </h1>
      <Link href="/" className="mt-6 text-[15px] text-chalk-2 underline underline-offset-4 hover:text-chalk">
        Back to the home page
      </Link>
    </div>
  );
}
