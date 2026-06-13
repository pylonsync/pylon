import React from "react";
import { type Metadata } from "@pylonsync/react";
import { Feed } from "./feed-client";

export const metadata: Metadata = {
  title: "__APP_NAME__ — a live social feed on Pylon",
  description:
    "A public feed with optimistic posts and likes, server-rendered over one Pylon backend. One binary, one port. Open two tabs and watch it sync.",
};

// `app/page.tsx` → `/`. The intro is server-rendered; `<Feed>` is a client
// island that mints a guest session and runs the live query + optimistic
// posts/likes in the browser.
export default function IndexPage() {
  return (
    <div>
      <header className="mb-6">
        <h1 className="text-2xl font-semibold tracking-tight">Feed</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Post something — it appears instantly and syncs to every tab. Likes
          are live too.
        </p>
      </header>
      <Feed />
    </div>
  );
}
