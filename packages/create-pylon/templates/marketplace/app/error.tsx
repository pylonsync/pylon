import React from "react";
import { type ErrorBoundaryProps } from "@pylonsync/react";

// `app/error.tsx` → the error boundary for this segment. Hydrated + interactive:
// `reset()` re-attempts the route. The thrown error reaches the client as
// `{ message, digest }` only — the stack stays in the dev overlay / server logs.
export default function Error({ error, reset }: ErrorBoundaryProps) {
  return (
    <div className="mx-auto flex min-h-[60vh] max-w-3xl flex-col items-center justify-center px-6 text-center">
      <h1 className="text-2xl font-semibold tracking-tight">Something went wrong</h1>
      <p className="mt-2 text-muted-foreground">{error.message}</p>
      {error.digest ? (
        <p className="mt-1 text-xs text-muted-foreground">
          Reference: <code>{error.digest}</code>
        </p>
      ) : null}
      <button
        type="button"
        onClick={reset}
        className="mt-6 inline-flex min-h-11 items-center rounded-lg bg-foreground px-5 text-sm font-medium text-background transition-[opacity,scale] duration-150 hover:opacity-90 active:scale-[0.96]"
      >
        Try again
      </button>
    </div>
  );
}
