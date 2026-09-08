import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";

// `not-found.tsx` inside the route group, so a missing page still renders
// with the site header and footer around it.
export default function NotFound(_props: NotFoundProps) {
  return (
    <div className="mx-auto max-w-3xl px-6 py-24 text-center">
      <h1 className="text-3xl font-semibold tracking-[-0.02em]">
        Page not found
      </h1>
      <p className="mt-3 text-[15px] text-ink-muted">
        That link does not go anywhere.
      </p>
      <Link
        href="/"
        className="mt-8 inline-block rounded-xl bg-ink px-5 py-3 text-[14px] font-medium text-surface transition-opacity hover:opacity-90"
      >
        Back to the home page
      </Link>
    </div>
  );
}
