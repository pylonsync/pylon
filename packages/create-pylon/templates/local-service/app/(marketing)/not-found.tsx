import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";

// `(marketing)/not-found.tsx` → rendered at HTTP 404 for any unmatched URL
// (and when a page calls `response.notFound()`). It lives inside the
// `(marketing)` group on purpose: a group segment adds no URL prefix, so this
// is still the root 404 boundary, but it wraps in the marketing layout — a
// missing URL gets the site nav + footer instead of a bare page. Hydrated, so
// the link is a client nav.
export default function NotFound(_props: NotFoundProps) {
  return (
    <div className="mx-auto flex min-h-[60vh] bg-cream text-ink max-w-3xl flex-col items-center justify-center px-6 text-center">
      <h1 className="font-display text-[5rem] leading-none">404</h1>
      <p className="mt-2 text-ink/70">We couldn&apos;t find that page.</p>
      <Link
        href="/"
        className="mt-6 inline-flex h-11 items-center bg-ink px-6 font-display text-[20px] tracking-[0.04em] text-cream transition-opacity hover:opacity-90"
      >
        Back home
      </Link>
    </div>
  );
}
