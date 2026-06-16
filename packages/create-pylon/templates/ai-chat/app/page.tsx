import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ChatApp } from "./chat-client";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// `app/page.tsx` → `/`. The whole app is the chat island — a sidebar of
// conversations + the streaming thread. Sign-in is REQUIRED: chats are tied to
// your account, so an unauthenticated visitor is redirected to /login (a real
// 307 from the synchronous shell render, before any HTML is sent).
export default function Home({ auth, response }: PageProps) {
  // Real account required — reject anonymous + guest (guest_…) sessions.
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  return <ChatApp />;
}
