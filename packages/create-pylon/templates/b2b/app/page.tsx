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
    "A server-rendered homepage, email/password auth, and a live client dashboard over one synced backend — one binary, one port.",
};

// `app/page.tsx` → `/`. This page is server-rendered: view source and the copy
// is in the HTML, not fetched later — good for SEO and first paint. It reads
// `auth` (resolved from the session cookie during the render) to show the
// right call to action. Every page receives `PageProps` from the SSR runtime:
// `{ url, params, searchParams, auth, response, serverData }` — typed, no
// hand-rolled interface.
export default function IndexPage({ auth }: PageProps) {
  const signedIn = Boolean(auth.user_id);
  return (
    <div className="space-y-12">
      <section className="space-y-5">
        <span className="inline-flex items-center rounded-full border px-3 py-1 text-xs font-medium text-muted-foreground">
          Server-rendered · authenticated · synced · one port
        </span>
        <h1 className="text-4xl font-semibold tracking-tight">
          Full-stack apps, one binary.
        </h1>
        <p className="max-w-xl text-lg text-muted-foreground">
          This homepage is server-rendered React. Sign in and your dashboard
          becomes a live, local-first view over the same Pylon backend — writes
          appear instantly and sync across tabs. No Next.js, no separate API
          server, no realtime sidecar.
        </p>
        <div className="flex flex-wrap items-center gap-3">
          {signedIn ? (
            <Button asChild>
              <Link href="/dashboard">Go to your dashboard →</Link>
            </Button>
          ) : (
            <>
              <Button asChild>
                <Link href="/signup">Get started</Link>
              </Button>
              <Button asChild variant="outline">
                <Link href="/login">Sign in</Link>
              </Button>
            </>
          )}
        </div>
      </section>

      <section className="grid gap-4 sm:grid-cols-3">
        <Feature title="Server-rendered">
          File-based routes under <Code>app/</Code>. Pages render to HTML on the
          server with <Code>metadata</Code> in <Code>{"<head>"}</Code>, then
          hydrate. Drop <Code>app/about/page.tsx</Code> to add{" "}
          <Code>/about</Code>.
        </Feature>
        <Feature title="Auth included">
          Email/password is built in. <Code>/login</Code> and{" "}
          <Code>/signup</Code> hit <Code>/api/auth/password/*</Code>; the server
          sets an HttpOnly session cookie. <Code>/dashboard</Code> gates on it
          server-side.
        </Feature>
        <Feature title="Synced database">
          Every <Code>entity()</Code> in <Code>app.ts</Code> gets a REST +
          realtime API and a typed client. <Code>db.useQuery</Code> is live;{" "}
          <Code>db.insert</Code> is optimistic.
        </Feature>
      </section>

      <p className="text-xs text-muted-foreground">
        Edit <Code>app/page.tsx</Code> and save — the page reloads instantly.
        The data model and access policies live in <Code>app.ts</Code>.
      </p>
    </div>
  );
}

function Feature({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">{title}</CardTitle>
      </CardHeader>
      <CardContent>
        <CardDescription className="text-sm leading-relaxed">
          {children}
        </CardDescription>
      </CardContent>
    </Card>
  );
}

function Code({ children }: { children: React.ReactNode }) {
  return <code className="rounded bg-muted px-1 text-xs">{children}</code>;
}
