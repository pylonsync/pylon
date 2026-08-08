import React from "react";
import { type ErrorBoundaryProps } from "@pylonsync/react";
import { Button } from "@/components/ui/button";

// `app/error.tsx` → the GLOBAL error boundary. Sitting at the app root
// (outside the `(marketing)` group), it catches a throw from any section —
// marketing, auth, or dashboard — and renders at HTTP 500 in the bare root
// shell, so it brings its own centered container. It's HYDRATED, so this is
// a real interactive client component: `reset()` re-attempts the route, and
// useState/onClick work. The thrown error reaches the client as
// `{ message, digest }` only — the stack stays in the dev overlay
// (PYLON_DEV_MODE) and the server logs, never in the page.
export default function Error({ error, reset }: ErrorBoundaryProps) {
  const [tries, setTries] = React.useState(0);
  return (
    <div className="mx-auto w-full max-w-xl space-y-6 px-6 py-24">
      <section>
        <h1 className="text-2xl font-semibold tracking-tight">
          Something went wrong
        </h1>
        <p className="mt-2 text-muted-foreground">{error.message}</p>
        {error.digest ? (
          <p className="mt-1 text-xs text-muted-foreground/70">
            Reference: <code>{error.digest}</code>
          </p>
        ) : null}
      </section>
      <div className="flex items-center gap-3">
        <Button
          onClick={() => {
            setTries((n) => n + 1);
            reset();
          }}
        >
          Try again
        </Button>
        {tries > 0 ? (
          <span className="text-sm text-muted-foreground">
            Retried {tries} {tries === 1 ? "time" : "times"}
          </span>
        ) : null}
      </div>
    </div>
  );
}
