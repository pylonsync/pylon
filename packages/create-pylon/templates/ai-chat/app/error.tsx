import React from "react";
import { type ErrorBoundaryProps } from "@pylonsync/react";

// The global error boundary. Renders at HTTP 500 for a throw anywhere in the
// app. The client receives `{ message, digest }` only; the stack stays in the
// server log.
export default function Error({ error, reset }: ErrorBoundaryProps) {
  return (
    <div className="flex min-h-[100dvh] flex-col items-center justify-center bg-bg px-6 text-center">
      <h1 className="font-serif text-[30px] tracking-[-0.02em] text-ink">Something went wrong</h1>
      <p className="mt-2 max-w-md text-[15px] text-ink-2">{error.message}</p>
      {error.digest ? (
        <p className="mt-1 text-[12.5px] text-ink-3">
          Reference: <code className="inline-code">{error.digest}</code>
        </p>
      ) : null}
      <button
        type="button"
        onClick={reset}
        className="press mt-7 inline-flex h-10 items-center rounded-full bg-ink px-5 text-[14px] font-medium text-bg transition-opacity hover:opacity-90"
      >
        Try again
      </button>
    </div>
  );
}
