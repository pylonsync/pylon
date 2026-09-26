import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { Console } from "../console";
import { ownerGate } from "../gate";

export const metadata: Metadata = { title: "Setup", robots: "noindex" };

// `/setup` — transport, model, owner, and assistant settings.
export default function SetupPage({ auth, response }: PageProps) {
  const blocked = ownerGate({ auth, response });
  if (blocked) return blocked;
  return <Console view="setup" initialConversationId={null} />;
}
