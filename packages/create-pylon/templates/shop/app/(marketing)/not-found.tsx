import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";
import { WRAP, BUTTON_DARK } from "@/components/marketing";

// `(marketing)/not-found.tsx` → rendered at HTTP 404 for any unmatched URL
// (and when a page calls `response.notFound()`). It lives inside the
// `(marketing)` group on purpose: a group segment adds no URL prefix, so this
// is still the root 404 boundary, but it wraps in the marketing layout, so a
// missing URL gets the site nav + footer. Hydrated, so the link is a client nav.
export default function NotFound(_props: NotFoundProps) {
  return (
    <section className={`${WRAP} flex min-h-[64vh] flex-col items-start justify-center py-24`}>
      <h1 className="font-display text-[3rem] font-light leading-none tracking-[-0.03em] text-ink sm:text-[4.25rem]">
        Page not found.
      </h1>
      <p className="mt-5 max-w-[40ch] text-[17px] leading-relaxed text-ink-2">
        The link may be old, or the product may be gone for good.
      </p>
      <Link href="/#shop" className={`${BUTTON_DARK} mt-9`}>
        Back to the shop
      </Link>
    </section>
  );
}
