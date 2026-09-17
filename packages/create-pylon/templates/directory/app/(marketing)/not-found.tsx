import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";
import { LINK, WRAP } from "@/components/marketing";

// `(marketing)/not-found.tsx` → rendered at HTTP 404 for any unmatched URL
// (and when a page calls `response.notFound()`). It lives inside the
// `(marketing)` group on purpose: a group segment adds no URL prefix, so this
// is still the root 404 boundary, but it wraps in the marketing layout, so a
// missing URL gets the site nav + footer instead of a bare page. Hydrated, so
// the link is a client nav.
export default function NotFound(_props: NotFoundProps) {
  return (
    <div className={`${WRAP} py-20`}>
      <p className="font-mono text-[12px] text-zinc-400">404</p>
      <h1 className="font-display mt-2 text-[22px] text-zinc-900">Page not found</h1>
      <p className="mt-2 text-[14px] text-zinc-600">
        There is no page at this address.{" "}
        <Link href="/" className={LINK}>
          Back to the directory
        </Link>
      </p>
    </div>
  );
}
