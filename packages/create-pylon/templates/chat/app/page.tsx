import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ChatRoom } from "./chat-client";

export const metadata: Metadata = {
  title: "__APP_NAME__",
  description: "A shared chat room. Everyone in the room sees the same messages.",
};

// `app/page.tsx` → `/`. `?c=<slug>` opens a channel (`/?c=launch`).
// `<ChatRoom>` is a client island: the server renders its skeleton, and the
// browser mints a guest session, then runs the live channels, messages, and
// presence.
export default function IndexPage({ searchParams }: PageProps) {
  return <ChatRoom initialChannel={searchParams.c} />;
}
