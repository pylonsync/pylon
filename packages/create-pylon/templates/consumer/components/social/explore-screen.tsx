"use client";

import React from "react";
import { GridSkeleton, PostGrid } from "./post-grid";
import { SocialGate } from "./session";
import { useSocial } from "./use-social";

export function ExploreScreen() {
  return (
    <div className="mx-auto max-w-[935px] px-0 py-2 sm:px-5 sm:py-8">
      <SocialGate fallback={<GridSkeleton count={12} />}>
        <LiveExplore />
      </SocialGate>
    </div>
  );
}

function LiveExplore() {
  const social = useSocial();
  if (social.loading) return <GridSkeleton count={12} />;
  if (social.posts.length === 0) return <p className="py-16 text-center text-sm text-zinc-500">No posts yet.</p>;
  return <PostGrid social={social} posts={social.posts} />;
}
