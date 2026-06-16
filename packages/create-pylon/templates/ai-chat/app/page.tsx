import React from "react";
import { type Metadata } from "@pylonsync/react";
import { ChatApp } from "./chat-client";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// `app/page.tsx` → `/`. The whole app is the chat island — a sidebar of
// conversations + the streaming thread. The layout renders the slim top bar; the
// island fills the rest of the viewport.
export default function Home() {
  return <ChatApp />;
}
