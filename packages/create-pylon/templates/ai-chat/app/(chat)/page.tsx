import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ChatApp } from "./chat-client";
import { siteConfig } from "@/lib/site.config";
import { brandIcon } from "@/lib/brand-icon";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
  icons: { icon: brandIcon },
};

// `/`: a new chat. No sign-in wall: a first-time visitor gets a guest session
// in the browser and can send a message right away.
export default function Home({ auth }: PageProps) {
  const signedIn = Boolean(auth.user_id) && !auth.is_guest && !auth.user_id?.startsWith("guest_");
  return <ChatApp initialConversationId={null} signedInUserId={signedIn ? auth.user_id : null} />;
}
