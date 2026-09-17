import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ProfileScreen } from "@/components/social/profile-screen";
import { SITE } from "@/lib/site";

export const metadata: Metadata = {
  title: `Profile · ${SITE.name}`,
};

// `app/(app)/u/[username]/page.tsx` → `/u/:username`. A person's posts,
// followers, and following. Your own profile also shows your saved posts.
export default function ProfilePage({ params }: PageProps<{ username: string }>) {
  return <ProfileScreen username={params.username} />;
}
