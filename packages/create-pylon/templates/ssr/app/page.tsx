import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

// SEO metadata. Export `metadata` (static) or `generateMetadata(props)`
// (dynamic) from any page or layout — Pylon renders the <title>/<meta>
// into <head> server-side. The `Metadata` type is exported from
// @pylonsync/react.
export const metadata: Metadata = {
  title: "__APP_NAME__ — full-stack Pylon app",
  description:
    "Server-rendered React, file-based routes, a synced database, and a typed client — one binary, one port.",
};

// `app/page.tsx` → `/`. Every page receives `PageProps` from the SSR
// runtime: `{ url, params, searchParams, auth, response, serverData }` —
// the type is exported from @pylonsync/react, no hand-rolled interface.
// This renders to HTML on the server; the per-route chunk hydrates it in
// the browser so interactive pages (see /counter) just work. shadcn/ui is
// pre-wired — `Button`/`Card` resolve through the `@/` alias; add more with
// `npx shadcn@latest add <component>`.
export default function IndexPage({ url }: PageProps) {
  return (
    <div className="space-y-8">
      <section>
        <h1 className="text-3xl font-semibold tracking-tight">__APP_NAME__</h1>
        <p className="mt-2 text-muted-foreground">
          A full-stack Pylon app. Server-rendered React, file-based routes,
          a synced database, and a typed client — served from one binary on
          one port. No Next.js, no separate API server. Styled with Tailwind
          v4 and shadcn/ui out of the box.
        </p>
      </section>

      <Card>
        <CardHeader>
          <CardTitle>Next steps</CardTitle>
          <CardDescription>
            Everything below is already wired — just edit the files.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ul className="space-y-2 text-sm text-foreground/80">
            <li>
              Add a route: drop a file at{" "}
              <code className="rounded bg-muted px-1">app/about/page.tsx</code>{" "}
              and visit <code className="rounded bg-muted px-1">/about</code>.
            </li>
            <li>
              Add data: edit{" "}
              <code className="rounded bg-muted px-1">app.ts</code> — every{" "}
              <code className="rounded bg-muted px-1">entity()</code> gets a
              REST + realtime API and a typed client automatically.
            </li>
            <li>
              Add a component:{" "}
              <code className="rounded bg-muted px-1">
                npx shadcn@latest add dialog
              </code>{" "}
              drops it into{" "}
              <code className="rounded bg-muted px-1">components/ui</code>.
            </li>
          </ul>
        </CardContent>
      </Card>

      <div className="flex flex-wrap items-center gap-3">
        <Button asChild>
          <Link href="/counter">See hydration in action →</Link>
        </Button>
        <Button asChild variant="outline">
          <Link href="/notes">Server data in the render →</Link>
        </Button>
        <Button asChild variant="ghost">
          <a
            href="https://docs.pylon.dev"
            target="_blank"
            rel="noreferrer noopener"
          >
            Read the docs
          </a>
        </Button>
      </div>

      <p className="text-xs text-muted-foreground">
        You're at <code>{url}</code>. Edit{" "}
        <code>app/page.tsx</code> and save — the page reloads instantly.
      </p>
    </div>
  );
}
