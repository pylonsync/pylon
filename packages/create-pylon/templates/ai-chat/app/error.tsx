import React from "react";
import { type ErrorBoundaryProps } from "@pylonsync/react";

// `app/error.tsx` → the GLOBAL error boundary. Sitting at the app root
// (outside the `(chat)` group), it catches a throw from any section — the chat
// or /login — and renders at HTTP 500 in the bare root shell. Hydrated +
// interactive: `reset()` re-attempts the route. The thrown error reaches the
// client as `{ message, digest }` only — the stack stays in the dev overlay /
// server logs.
export default function Error({ error, reset }: ErrorBoundaryProps) {
  return (
    <div className="mx-auto flex min-h-[60vh] max-w-3xl flex-col items-center justify-center px-6 text-center">
      <h1 className="text-2xl font-semibold tracking-tight">Something went wrong</h1>
      <p className="mt-2 text-zinc-500">{error.message}</p>
      {error.digest ? (
        <p className="mt-1 text-xs text-zinc-400">
          Reference: <code>{error.digest}</code>
        </p>
      ) : null}
      <button
        type="button"
        onClick={reset}
        className="mt-6 inline-flex h-10 items-center rounded-full bg-zinc-900 px-5 text-sm font-medium text-white transition-colors hover:bg-zinc-700"
      >
        Try again
      </button>
    </div>
  );
}
