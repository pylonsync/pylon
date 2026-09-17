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
    <div className="mx-auto w-full max-w-[40rem] px-6 py-24">
      <h1 className="text-[2rem] font-medium leading-tight text-ink">Page not found</h1>
      <p className="mt-4 text-[17.5px] leading-[1.65] text-ink-2">
        There is nothing at this address.
      </p>
      <Link
        href="/"
        className="mt-6 inline-block text-[17.5px] font-medium text-ink underline decoration-brand/50 underline-offset-4 hover:decoration-brand"
      >
        Back to the home page
      </Link>
    </div>
  );
}
