import React from "react";
import { type Metadata } from "@pylonsync/react";
import { TodoApp } from "./todo-app";

// SEO metadata. Export `metadata` (static) or `generateMetadata(props)`
// (dynamic) from any page or layout — Pylon renders the <title>/<meta>
// into <head> server-side. The `Metadata` type is exported from
// @pylonsync/react.
export const metadata: Metadata = {
  title: "__APP_NAME__ — a live Pylon todo",
  description:
    "A server-rendered todo list with live, optimistic, per-user sync. Open two tabs and watch them stay in sync.",
};

// `app/page.tsx` → `/`. The heading and intro are server-rendered (view
// source and they're in the HTML — good for SEO and first paint). The list
// itself is a client island: `<TodoApp>` mints a guest session and runs the
// live query + optimistic writes in the browser.
export default function IndexPage() {
  return (
    <div className="flex flex-1 flex-col">
      <header className="mb-8">
        <h1 className="text-3xl font-semibold tracking-tight">Todos</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Live and optimistic over one Pylon backend. Open a second tab —
          changes sync instantly.
        </p>
      </header>
      <TodoApp />
      <p className="mt-8 text-xs text-muted-foreground">
        Edit <code className="rounded bg-muted px-1">app/todo-app.tsx</code> for
        the UI, or <code className="rounded bg-muted px-1">app.ts</code> for the
        data model and access policies.
      </p>
    </div>
  );
}
