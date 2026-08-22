import React from "react";
import { Link, type NotFoundProps } from "@pylonsync/react";
import { Button } from "@/components/ui/button";

// `app/not-found.tsx` → rendered at HTTP 404 for any unmatched URL, and when a
// page calls `response.notFound()`.
//
// Give it real links. This boundary is also what an agent reads when it guesses
// a URL wrong: a markdown request for a missing page renders THIS file as
// markdown at 404, so the links below are how it finds its way back.
export default function NotFound(_props: NotFoundProps) {
  return (
    <div className="mx-auto flex min-h-[60vh] max-w-lg flex-col justify-center gap-6 px-4">
      <section>
        <h1 className="text-2xl font-semibold tracking-tight">Page not found</h1>
        <p className="mt-2 text-muted-foreground">
          That URL does not exist. Try one of these:
        </p>
      </section>
      <ul className="space-y-2 text-sm">
        <li>
          <Link className="underline underline-offset-4" href="/">
            Home
          </Link>
        </li>
        <li>
          <a className="underline underline-offset-4" href="/sitemap.xml">
            Sitemap
          </a>
        </li>
        <li>
          <a className="underline underline-offset-4" href="/llms.txt">
            llms.txt
          </a>
        </li>
      </ul>
      <div>
        <Button asChild>
          <Link href="/">Back to home</Link>
        </Button>
      </div>
    </div>
  );
}
