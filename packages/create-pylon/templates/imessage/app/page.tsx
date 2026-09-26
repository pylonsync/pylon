import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { Console } from "./console";
import { ownerGate } from "./gate";

export const metadata: Metadata = { title: "Messages", robots: "noindex" };

// `/` — conversations. `?c=<conversation id>` opens one thread.
export default function MessagesPage({ auth, response, searchParams }: PageProps) {
  const blocked = ownerGate({ auth, response });
  if (blocked) return blocked;
  const c = typeof searchParams?.c === "string" ? searchParams.c : null;
  return <Console view="messages" initialConversationId={c} />;
}
