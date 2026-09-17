import React from "react";
import { type Metadata } from "@pylonsync/react";
import { HomeScreen } from "@/components/social/home-screen";
import { SITE } from "@/lib/site";

export const metadata: Metadata = {
  title: SITE.name,
  description: SITE.description,
};

// `app/(app)/page.tsx` → `/`. The feed is a client island: it needs the
// visitor's session to know who they follow and what they liked.
export default function HomePage() {
  return <HomeScreen />;
}
