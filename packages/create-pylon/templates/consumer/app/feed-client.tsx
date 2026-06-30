"use client";

import React, { useEffect, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest, useAuth } from "@pylonsync/client";
import { Button } from "@/components/ui/button";

export interface Post {
  id: string;
  authorId: string;
  text: string;
  createdAt: string;
}

export interface Like {
  id: string;
  userId: string;
  postId: string;
}

// `<EnsureGuest>` mints a guest session so anyone can post + like with no
// login. Inside it, `useAuth()` exposes the guest's `userId` — that's how we
// know which posts the current visitor has already liked.
export function Feed() {
  return (
    <EnsureGuest fallback={<FeedSkeleton />}>
      <FeedInner />
    </EnsureGuest>
  );
}

function FeedInner() {
  const { userId } = useAuth();
  const [text, setText] = useState("");
  // Seed a few demo posts on first visit so the feed isn't empty. No-op once
  // any post exists; safe to call every mount.
  useEffect(() => {
    void callFn("seedPosts", {});
  }, []);
  const { data: posts, loading } = db.useQuery<Post>("Post");
  // Public-read, so this is every like in the system — we count + match them
  // client-side. db.useQuery is live, so counts update the instant anyone likes.
  const { data: likes } = db.useQuery<Like>("Like");

  async function addPost(e: React.FormEvent) {
    e.preventDefault();
    const value = text.trim();
    if (!value) return;
    setText("");
    // No authorId sent — field.owner() stamps it from the session.
    await db.insert("Post", { text: value });
  }

  function toggleLike(postId: string) {
    const mine = likes.find((l) => l.postId === postId && l.userId === userId);
    if (mine) {
      void db.delete("Like", mine.id);
    } else {
      void db.insert("Like", { postId });
    }
  }

  const ordered = [...posts].sort((a, b) =>
    b.createdAt.localeCompare(a.createdAt),
  );

  return (
    <div className="space-y-6">
      <form onSubmit={addPost} className="space-y-2">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="What's happening?"
          aria-label="Post"
          rows={2}
          className="w-full resize-none rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        <div className="flex justify-end">
          <Button type="submit" disabled={!text.trim()}>
            Post
          </Button>
        </div>
      </form>

      {loading && posts.length === 0 ? (
        <FeedSkeleton />
      ) : ordered.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No posts yet — say something above. It appears instantly and syncs;
          open a second tab to watch the feed update live.
        </p>
      ) : (
        <ul className="space-y-3">
          {ordered.map((post) => {
            const count = likes.filter((l) => l.postId === post.id).length;
            const liked = likes.some(
              (l) => l.postId === post.id && l.userId === userId,
            );
            const mine = post.authorId === userId;
            return (
              <li key={post.id} className="rounded-lg border px-4 py-3">
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span className="font-mono">
                    {mine ? "you" : shortId(post.authorId)}
                  </span>
                  {mine && (
                    <button
                      type="button"
                      aria-label="Delete post"
                      onClick={() => db.delete("Post", post.id)}
                      className="hover:text-red-600"
                    >
                      Delete
                    </button>
                  )}
                </div>
                <p className="mt-1 whitespace-pre-wrap text-sm">{post.text}</p>
                <div className="mt-2">
                  <button
                    type="button"
                    onClick={() => toggleLike(post.id)}
                    aria-pressed={liked}
                    className={
                      "inline-flex items-center gap-1.5 text-xs transition-colors " +
                      (liked
                        ? "text-rose-600"
                        : "text-muted-foreground hover:text-foreground")
                    }
                  >
                    <span>{liked ? "♥" : "♡"}</span>
                    <span>{count}</span>
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

// Guest ids look like `guest_<hex>`; show a short, stable handle.
function shortId(id: string) {
  return "@" + id.replace(/^guest_/, "").slice(0, 6);
}

function FeedSkeleton() {
  return (
    <ul className="space-y-3" aria-hidden>
      {[0, 1, 2].map((i) => (
        <li key={i} className="rounded-lg border px-4 py-3">
          <span className="block h-3 w-16 rounded bg-muted" />
          <span className="mt-2 block h-4 w-3/4 rounded bg-muted" />
        </li>
      ))}
    </ul>
  );
}
