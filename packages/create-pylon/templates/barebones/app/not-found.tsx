import React from "react";
import { Link, useRouter, type NotFoundProps } from "@pylonsync/react";
import { Button } from "@/components/ui/button";

// `app/not-found.tsx` → rendered at HTTP 404 for any unmatched URL (and when
// a page calls `response.notFound()`). It's HYDRATED, so it's interactive:
// the buttons below use the client router. Not-found boundaries receive the
// standard page props (and, matching Next, no `reset`).
export default function NotFound(_props: NotFoundProps) {
  const router = useRouter();
  return (
    <div className="mx-auto max-w-lg space-y-6 px-4 py-12">
      <section>
        <h1 className="text-2xl font-semibold tracking-tight">404</h1>
        <p className="mt-2 text-muted-foreground">
          We couldn&apos;t find that page.
        </p>
      </section>
      <div className="flex items-center gap-3">
        <Button onClick={() => router.back()} variant="outline">
          ← Go back
        </Button>
        <Button render={<Link href="/" />}>Home</Button>
      </div>
    </div>
  );
}
