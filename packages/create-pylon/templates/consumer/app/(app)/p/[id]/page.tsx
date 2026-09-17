import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { PostScreen } from "@/components/social/post-screen";
import { SITE } from "@/lib/site";

export const metadata: Metadata = {
  title: `Post · ${SITE.name}`,
};

// `app/(app)/p/[id]/page.tsx` → `/p/:id`. One post with all its comments.
export default function PostPage({ params }: PageProps<{ id: string }>) {
  return <PostScreen postId={params.id} />;
}
