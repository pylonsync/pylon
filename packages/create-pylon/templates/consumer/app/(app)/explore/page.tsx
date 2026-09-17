import React from "react";
import { type Metadata } from "@pylonsync/react";
import { ExploreScreen } from "@/components/social/explore-screen";
import { SITE } from "@/lib/site";

export const metadata: Metadata = {
  title: `Explore · ${SITE.name}`,
};

// `app/(app)/explore/page.tsx` → `/explore`. Every post, newest first.
export default function ExplorePage() {
  return <ExploreScreen />;
}
