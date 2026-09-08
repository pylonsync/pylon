import React from "react";
import { type ErrorBoundaryProps } from "@pylonsync/react";

// The error boundary sits at the root, outside the route group, so it still
// renders if a layout itself throws. It is hydrated, so `reset()` works.
export default function ErrorBoundary({ error, reset }: ErrorBoundaryProps) {
  return (
    <div className="mx-auto max-w-3xl px-6 py-24 text-center">
      <h1 className="text-3xl font-semibold tracking-[-0.02em]">
        Something went wrong
      </h1>
      <p className="mt-3 text-[15px] text-ink-muted">{error.message}</p>
      {error.digest ? (
        <p className="mt-1 text-[12px] text-ink-muted">
          Reference: <code className="font-mono">{error.digest}</code>
        </p>
      ) : null}
      <button
        type="button"
        onClick={reset}
        className="mt-8 rounded-xl bg-ink px-5 py-3 text-[14px] font-medium text-surface transition-opacity hover:opacity-90"
      >
        Try again
      </button>
    </div>
  );
}
