import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";

// Rendered at HTTP 404 for any unmatched URL.
export default function NotFound(_props: NotFoundProps) {
  return (
    <div className="flex min-h-[100dvh] flex-col items-center justify-center bg-bg px-6 text-center">
      <h1 className="font-serif text-[34px] tracking-[-0.02em] text-ink">Page not found</h1>
      <p className="mt-2 text-[15px] text-ink-2">That address does not match any page.</p>
      <Link
        href="/"
        className="press mt-7 inline-flex h-10 items-center rounded-full bg-ink px-5 text-[14px] font-medium text-bg transition-opacity hover:opacity-90"
      >
        Start a new chat
      </Link>
    </div>
  );
}
