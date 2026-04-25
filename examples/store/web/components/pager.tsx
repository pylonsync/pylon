"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Button } from "@pylonsync/example-ui/button";

export function Pager({
  page,
  totalPages,
}: {
  page: number;
  totalPages: number;
}) {
  const router = useRouter();
  const params = useSearchParams();

  const goto = (next: number) => {
    const sp = new URLSearchParams(params.toString());
    if (next <= 0) sp.delete("page");
    else sp.set("page", String(next));
    router.push(`/?${sp.toString()}`, { scroll: false });
  };

  return (
    <div className="mt-4 flex items-center justify-center gap-4 text-sm text-muted-foreground">
      <Button
        variant="outline"
        size="sm"
        disabled={page === 0}
        onClick={() => goto(page - 1)}
      >
        ← Previous
      </Button>
      <span>
        Page {page + 1} of {totalPages}
      </span>
      <Button
        variant="outline"
        size="sm"
        disabled={page + 1 >= totalPages}
        onClick={() => goto(page + 1)}
      >
        Next →
      </Button>
    </div>
  );
}
