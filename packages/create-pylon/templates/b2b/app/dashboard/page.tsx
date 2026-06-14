import React, { Suspense, use } from "react";
import {
  type Metadata,
  type PageProps,
  type ServerData,
} from "@pylonsync/react";
import { Workspace } from "./dashboard-client";

export const metadata: Metadata = {
  title: "Dashboard — __APP_NAME__",
  robots: "noindex",
};

interface User {
  id: string;
  email: string;
  displayName?: string;
}

// Reads the signed-in user DURING the server render (owner-scoped: only your
// own row). The org list + the active org's projects load client-side, since
// they depend on the session's selected tenant.
function Greeting({
  serverData,
  userId,
}: {
  serverData: ServerData;
  userId: string;
}) {
  const user = use(serverData.get<User>("User", userId));
  return (
    <p className="text-sm text-muted-foreground">
      Signed in as{" "}
      <span className="font-medium text-foreground">
        {user?.displayName || user?.email || "you"}
      </span>
      .
    </p>
  );
}

// `app/dashboard/page.tsx` → `/dashboard`.
export default function DashboardPage({
  auth,
  response,
  serverData,
}: PageProps) {
  // Server-side auth gate: anonymous requests get a 307 to /login before any
  // HTML. Must fire in the synchronous shell render, not inside <Suspense> —
  // and return immediately so nothing renders below the already-sent redirect.
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  return (
    <div className="space-y-6">
      <Suspense fallback={null}>
        <Greeting serverData={serverData} userId={auth.user_id!} />
      </Suspense>
      <Workspace />
    </div>
  );
}
