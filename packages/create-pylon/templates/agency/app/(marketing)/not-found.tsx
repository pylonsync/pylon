import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";
import { WRAP_NARROW, TEXT_LINK } from "@/components/marketing";

// `(marketing)/not-found.tsx` → rendered at HTTP 404 for any unmatched URL
// (and when a page calls `response.notFound()`). It lives inside the
// `(marketing)` group on purpose: a group segment adds no URL prefix, so this
// is still the root 404 boundary, but it wraps in the marketing layout — a
// missing URL gets the site nav + footer instead of a bare page. Hydrated, so
// the link is a client nav.
export default function NotFound(_props: NotFoundProps) {
  return (
    <div className={`${WRAP_NARROW} flex min-h-[60vh] flex-col justify-center py-20 text-ink`}>
      <h1 className="font-display text-[2.75rem] leading-none">Page not found</h1>
      <p className="mt-4 text-[16px] text-zinc-600">There is nothing at this address.</p>
      <Link href="/" className={`mt-6 inline-block self-start text-[15px] ${TEXT_LINK}`}>
        Back to the homepage
      </Link>
    </div>
  );
}
