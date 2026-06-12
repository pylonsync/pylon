import React, { Suspense, use } from "react";
import {
  type Metadata,
  type PageProps,
  type ServerData,
} from "@pylonsync/react";
import { Dashboard, type Note } from "./dashboard-client";

export const metadata: Metadata = {
  title: "Dashboard — __APP_NAME__",
  robots: "noindex",
};

interface User {
  id: string;
  email: string;
  displayName?: string;
}

// Reads the signed-in user + their notes DURING the server render. The reads
// run through the same policy gate as a client query, so they're owner-scoped
// (User: only your row; Note: only your notes) — see the policies in app.ts.
// React 19 `use()` suspends until they resolve on the server, so the HTML
// arrives with your notes already in it (no empty flash); then the <Dashboard>
// island hydrates and takes over live.
function DashboardBody({
  serverData,
  userId,
}: {
  serverData: ServerData;
  userId: string;
}) {
  const user = use(serverData.get<User>("User", userId));
  const notes = use(serverData.list<Note>("Note"));
  return (
    <>
      <p className="text-sm text-muted-foreground">
        Signed in as{" "}
        <span className="font-medium text-foreground">
          {user?.displayName || user?.email || "you"}
        </span>
        .
      </p>
      <Dashboard initial={notes} />
    </>
  );
}

// `app/dashboard/page.tsx` → `/dashboard`.
export default function DashboardPage({
  auth,
  response,
  serverData,
}: PageProps) {
  // Server-side auth gate: anonymous requests get a 307 to /login before any
  // HTML. The redirect MUST fire here in the synchronous shell render — not
  // inside the <Suspense> below — or React swallows it. No flash of the
  // dashboard, works with JS disabled.
  if (!auth.user_id) response.redirect("/login");
  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-semibold tracking-tight">Your notes</h1>
      <Suspense
        fallback={<p className="text-sm text-muted-foreground">Loading…</p>}
      >
        <DashboardBody serverData={serverData} userId={auth.user_id!} />
      </Suspense>
    </div>
  );
}
