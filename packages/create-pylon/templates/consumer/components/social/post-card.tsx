"use client";

import React from "react";
import { Link } from "@pylonsync/react";
import { plural, timeAgo, type Post } from "@/lib/social";
import { cn } from "@/lib/utils";
import { Avatar } from "./avatar";
import { CommentForm } from "./comment-form";
import { LikesLine, PostActions, PostMenu, PostPhoto } from "./post-parts";
import { RichText } from "./rich-text";
import type { Social } from "./use-social";

/** One post in the home feed. */
export function PostCard({ social, post }: { social: Social; post: Post }) {
  const author = social.profileByUser.get(post.authorId);
  const comments = social.commentsByPost.get(post.id) ?? [];
  const shown = comments.slice(-2);
  const username = author?.username ?? "unknown";

  return (
    <article className="border-b pb-4 sm:pb-5">
      <header className="flex items-center gap-2.5 px-3 py-3 sm:px-0">
        <Link href={`/u/${username}`}>
          <Avatar profile={author} size={32} />
        </Link>
        <div className="flex min-w-0 items-baseline gap-1.5 text-sm">
          <Link href={`/u/${username}`} className="truncate font-semibold hover:text-zinc-500">
            {username}
          </Link>
          <span className="text-zinc-500">·</span>
          <Link href={`/p/${post.id}`} className="shrink-0 text-zinc-500 hover:underline">
            <time dateTime={post.createdAt}>{timeAgo(post.createdAt)}</time>
          </Link>
        </div>
        <div className="ml-auto">
          <PostMenu social={social} post={post} />
        </div>
      </header>

      <PostPhoto
        social={social}
        post={post}
        className={cn("w-full sm:rounded-[4px] sm:border", post.shape === "portrait" ? "aspect-[4/5]" : "aspect-square")}
      />

      <div className="px-3 pt-1 sm:px-0">
        <PostActions social={social} post={post} />
        <LikesLine social={social} post={post} />
        {post.caption ? (
          <p className="mt-1.5 text-sm leading-snug">
            <Link href={`/u/${username}`} className="mr-1.5 font-semibold">
              {username}
            </Link>
            <RichText text={post.caption} />
          </p>
        ) : null}
        {comments.length > shown.length ? (
          <Link href={`/p/${post.id}`} className="mt-1.5 block text-sm text-zinc-500">
            View all {plural(comments.length, "comment")}
          </Link>
        ) : null}
        {shown.map((c) => {
          const who = social.profileByUser.get(c.authorId)?.username ?? "unknown";
          return (
            <p key={c.id} className="mt-1 text-sm leading-snug">
              <Link href={`/u/${who}`} className="mr-1.5 font-semibold">
                {who}
              </Link>
              <RichText text={c.text} />
            </p>
          );
        })}
        <CommentForm postId={post.id} className="mt-1" />
      </div>
    </article>
  );
}
