import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";

// `app/not-found.tsx` → rendered at HTTP 404 for any unmatched URL (and when a
// page calls `response.notFound()` — e.g. a listing slug that doesn't resolve).
// Hydrated, so the link is a client nav.
export default function NotFound(_props: NotFoundProps) {
  return (
    <Empty className="mx-auto min-h-[60vh] max-w-3xl border-0 px-6">
      <EmptyHeader>
        <EmptyTitle className="text-3xl">404</EmptyTitle>
        <EmptyDescription>We couldn&apos;t find that page.</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button asChild>
          <Link href="/">Back to browse</Link>
        </Button>
      </EmptyContent>
    </Empty>
  );
}
