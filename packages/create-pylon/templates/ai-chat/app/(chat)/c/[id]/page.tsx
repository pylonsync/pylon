import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ChatApp } from "../../chat-client";
import { siteConfig } from "@/lib/site.config";
import { brandIcon } from "@/lib/brand-icon";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  robots: "noindex",
  icons: { icon: brandIcon },
};

// `/c/:id`: one conversation. The id is only a starting point; the client
// loads the conversation through the owner-scoped live query, so another
// visitor's id shows nothing.
export default function ConversationPage({ auth, params }: PageProps<{ id: string }>) {
  const signedIn = Boolean(auth.user_id) && !auth.is_guest && !auth.user_id?.startsWith("guest_");
  return <ChatApp initialConversationId={params.id} signedInUserId={signedIn ? auth.user_id : null} />;
}
