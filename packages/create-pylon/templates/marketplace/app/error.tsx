import React from "react";
import { type ErrorBoundaryProps } from "@pylonsync/react";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";

// `app/error.tsx` → the error boundary for this segment. Hydrated + interactive:
// `reset()` re-attempts the route. The thrown error reaches the client as
// `{ message, digest }` only — the stack stays in the dev overlay / server logs.
export default function Error({ error, reset }: ErrorBoundaryProps) {
  return (
    <Empty className="mx-auto min-h-[60vh] max-w-3xl border-0 px-6">
      <EmptyHeader>
        <EmptyTitle className="text-2xl">Something went wrong</EmptyTitle>
        <EmptyDescription className="max-w-md">
          Reprise could not load this page. Try again, or return to browse if
          the problem continues.
        </EmptyDescription>
        {error.digest ? (
          <p className="text-xs text-muted-foreground">
            Reference: <code>{error.digest}</code>
          </p>
        ) : null}
      </EmptyHeader>
      <EmptyContent>
        <Button type="button" onClick={reset}>
          Try again
        </Button>
        <Button asChild variant="link">
          <a href="/">Back to browse</a>
        </Button>
      </EmptyContent>
    </Empty>
  );
}
