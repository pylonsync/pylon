"use client";

import React from "react";
import { Link } from "@pylonsync/react";
import { Heart, MessageCircle } from "lucide-react";
import { formatCount, type Post } from "@/lib/social";
import type { Social } from "./use-social";

/** Square tiles, three across. Hover shows the like and comment counts. */
export function PostGrid({ social, posts }: { social: Social; posts: Post[] }) {
  return (
    <ul className="grid grid-cols-3 gap-1">
      {posts.map((post) => {
        const likes = social.likeCount.get(post.id) ?? 0;
        const comments = social.commentsByPost.get(post.id)?.length ?? 0;
        return (
          <li key={post.id}>
            <Link href={`/p/${post.id}`} className="group relative block aspect-square overflow-hidden bg-zinc-100">
              <img src={post.imageUrl} alt={post.caption ?? ""} loading="lazy" className="size-full object-cover" />
              <span className="absolute inset-0 hidden items-center justify-center gap-6 bg-black/35 text-base font-bold text-white group-hover:flex">
                <span className="flex items-center gap-1.5">
                  <Heart className="size-5 fill-white" />
                  {formatCount(likes)}
                </span>
                <span className="flex items-center gap-1.5">
                  <MessageCircle className="size-5 -scale-x-100 fill-white" />
                  {formatCount(comments)}
                </span>
              </span>
            </Link>
          </li>
        );
      })}
    </ul>
  );
}

export function GridSkeleton({ count = 9 }: { count?: number }) {
  return (
    <ul className="grid grid-cols-3 gap-1" aria-hidden>
      {Array.from({ length: count }, (_, i) => (
        <li key={i} className="aspect-square bg-zinc-100" />
      ))}
    </ul>
  );
}
