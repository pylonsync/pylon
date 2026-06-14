import React from "react";
import { type Metadata } from "@pylonsync/react";
import { ChatRoom } from "./chat-client";

export const metadata: Metadata = {
  title: "__APP_NAME__ — realtime chat on Pylon",
  description:
    "A live chat room over one Pylon backend. Open two tabs and watch messages sync instantly.",
};

// `app/page.tsx` → `/`. The header is server-rendered; `<ChatRoom>` is a client
// island that mints a guest session and runs the live message subscription +
// optimistic send in the browser.
export default function IndexPage() {
  return (
    <>
      <header className="shrink-0 py-4">
        <h1 className="text-xl font-semibold tracking-tight">Room</h1>
        <p className="text-sm text-muted-foreground">
          One shared room. Open a second tab — messages sync instantly.
        </p>
      </header>
      <ChatRoom />
    </>
  );
}
