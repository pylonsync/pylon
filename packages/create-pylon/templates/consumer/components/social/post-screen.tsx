"use client";

import React, { useRef } from "react";
import { db, Link } from "@pylonsync/react";
import { X } from "lucide-react";
import { timeAgo, type Comment } from "@/lib/social";
import { cn } from "@/lib/utils";
import { Avatar } from "./avatar";
import { CommentForm } from "./comment-form";
import { LikesLine, PostActions, PostMenu, PostPhoto } from "./post-parts";
import { PostGrid } from "./post-grid";
import { RichText } from "./rich-text";
import { SocialGate } from "./session";
import { actions, useSocial, type Social } from "./use-social";

export function PostScreen({ postId }: { postId: string }) {
  return (
    <div className="mx-auto max-w-[935px] py-0 sm:px-5 sm:py-8">
      <SocialGate fallback={<div className="aspect-[4/3] w-full bg-zinc-100 sm:rounded-md" />}>
        <LivePost postId={postId} />
      </SocialGate>
    </div>
  );
}

function LivePost({ postId }: { postId: string }) {
  const social = useSocial();
  const commentInput = useRef<HTMLDivElement | null>(null);
  const post = social.posts.find((p) => p.id === postId);
  if (!post) {
    return social.loading ? (
      <div className="aspect-[4/3] w-full bg-zinc-100 sm:rounded-md" />
    ) : (
      <div className="py-24 text-center">
        <p className="text-lg font-semibold">This post is not available.</p>
        <p className="mt-2 text-sm text-zinc-500">It may have been deleted.</p>
        <Link href="/" className="mt-4 inline-block text-sm font-semibold text-brand">
          Back to the feed
        </Link>
      </div>
    );
  }

  const author = social.profileByUser.get(post.authorId);
  const username = author?.username ?? "unknown";
  const comments = social.commentsByPost.get(post.id) ?? [];
  const more = (social.postsByAuthor.get(post.authorId) ?? []).filter((p) => p.id !== post.id).slice(0, 6);
  const mine = post.authorId === social.me;
  const followingAuthor = social.following.has(post.authorId);

  return (
    <>
      <article className="flex flex-col overflow-hidden border-y bg-background sm:rounded-md sm:border md:h-[min(86vh,760px)] md:flex-row">
        <PostPhoto
          social={social}
          post={post}
          fit="contain"
          className={cn("bg-black md:h-full md:flex-1", post.shape === "portrait" ? "aspect-[4/5] md:aspect-auto" : "aspect-square md:aspect-auto")}
        />
        <div className="flex min-h-0 flex-col md:w-[400px] md:shrink-0 md:border-l">
          <header className="flex items-center gap-3 border-b px-4 py-3">
            <Link href={`/u/${username}`}>
              <Avatar profile={author} size={32} />
            </Link>
            <Link href={`/u/${username}`} className="text-sm font-semibold hover:text-zinc-500">
              {username}
            </Link>
            {!mine && social.me ? (
              <>
                <span className="text-zinc-400">·</span>
                <button
                  type="button"
                  onClick={() => actions.toggleFollow(social, post.authorId)}
                  className={cn("text-sm font-semibold", followingAuthor ? "text-zinc-900" : "text-brand hover:text-zinc-900")}
                >
                  {followingAuthor ? "Following" : "Follow"}
                </button>
              </>
            ) : null}
            <div className="ml-auto">
              <PostMenu social={social} post={post} />
            </div>
          </header>

          <ul className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 py-4">
            {post.caption ? (
              <CommentRow social={social} authorId={post.authorId} text={post.caption} createdAt={post.createdAt} />
            ) : null}
            {comments.map((c) => (
              <CommentRow key={c.id} social={social} authorId={c.authorId} text={c.text} createdAt={c.createdAt} comment={c} />
            ))}
            {!post.caption && comments.length === 0 ? (
              <li className="m-auto py-8 text-center">
                <p className="text-lg font-semibold">No comments yet.</p>
                <p className="mt-1 text-sm text-zinc-500">Start the conversation.</p>
              </li>
            ) : null}
          </ul>

          <div className="border-t px-4 pt-1">
            <PostActions social={social} post={post} onComment={() => commentInput.current?.querySelector("input")?.focus()} />
            <LikesLine social={social} post={post} />
            <time dateTime={post.createdAt} className="mt-1 block pb-3 text-xs text-zinc-500">
              {new Date(post.createdAt).toLocaleDateString("en-US", { month: "long", day: "numeric", year: "numeric" })}
            </time>
          </div>
          <div ref={commentInput} className="border-t px-4">
            <CommentForm postId={post.id} />
          </div>
        </div>
      </article>

      {more.length ? (
        <section className="mt-12 border-t pt-8">
          <p className="mb-4 px-3 text-sm font-semibold text-zinc-500 sm:px-0">
            More posts from{" "}
            <Link href={`/u/${username}`} className="text-zinc-900 hover:underline">
              {username}
            </Link>
          </p>
          <PostGrid social={social} posts={more} />
        </section>
      ) : null}
    </>
  );
}

function CommentRow({
  social,
  authorId,
  text,
  createdAt,
  comment,
}: {
  social: Social;
  authorId: string;
  text: string;
  createdAt: string;
  comment?: Comment;
}) {
  const who = social.profileByUser.get(authorId);
  const username = who?.username ?? "unknown";
  const mine = comment && comment.authorId === social.me;
  return (
    <li className="group flex gap-3">
      <Link href={`/u/${username}`} className="shrink-0">
        <Avatar profile={who} size={32} />
      </Link>
      <div className="min-w-0 flex-1 text-sm leading-snug">
        <p className="break-words">
          <Link href={`/u/${username}`} className="mr-1.5 font-semibold">
            {username}
          </Link>
          <RichText text={text} />
        </p>
        <p className="mt-1 flex items-center gap-3 text-xs text-zinc-500">
          <time dateTime={createdAt}>{timeAgo(createdAt)}</time>
          {mine ? (
            <button
              type="button"
              onClick={() => void db.delete("Comment", comment.id)}
              className="hidden items-center gap-1 hover:text-zinc-900 group-hover:inline-flex"
              aria-label="Delete comment"
            >
              <X className="size-3" /> Delete
            </button>
          ) : null}
        </p>
      </div>
    </li>
  );
}

