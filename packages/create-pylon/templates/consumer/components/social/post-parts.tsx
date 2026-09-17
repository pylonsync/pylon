"use client";

import React, { useState } from "react";
import { db, Link, useRouter } from "@pylonsync/react";
import { Bookmark, Heart, MessageCircle, MoreHorizontal, Send } from "lucide-react";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { plural, type Post } from "@/lib/social";
import { cn } from "@/lib/utils";
import { actions, type Social } from "./use-social";

/** Heart, comment, share, and save, in one row. */
export function PostActions({ social, post, onComment }: { social: Social; post: Post; onComment?: () => void }) {
  const liked = social.myLikes.has(post.id);
  const saved = social.mySaves.has(post.id);
  const [copied, setCopied] = useState(false);

  async function share() {
    const url = `${window.location.origin}/p/${post.id}`;
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      window.prompt("Copy this link", url);
    }
  }

  return (
    <div className="-mx-2 flex items-center">
      <IconButton label={liked ? "Unlike" : "Like"} onClick={() => actions.toggleLike(social, post.id)}>
        <Heart className={cn("size-6 transition-transform active:scale-90", liked ? "fill-like text-like" : "")} strokeWidth={liked ? 2 : 1.75} />
      </IconButton>
      {onComment ? (
        <IconButton label="Comment" onClick={onComment}>
          <MessageCircle className="size-6 -scale-x-100" strokeWidth={1.75} />
        </IconButton>
      ) : (
        <Link href={`/p/${post.id}`} aria-label="Comment" className="p-2 hover:opacity-60">
          <MessageCircle className="size-6 -scale-x-100" strokeWidth={1.75} />
        </Link>
      )}
      <IconButton label="Copy link" onClick={share}>
        <Send className="size-6" strokeWidth={1.75} />
      </IconButton>
      {copied ? <span className="text-xs text-zinc-500">Link copied</span> : null}
      <IconButton label={saved ? "Remove from saved" : "Save"} onClick={() => actions.toggleSave(social, post.id)} className="ml-auto">
        <Bookmark className={cn("size-6", saved ? "fill-current" : "")} strokeWidth={1.75} />
      </IconButton>
    </div>
  );
}

export function LikesLine({ social, post }: { social: Social; post: Post }) {
  const count = social.likeCount.get(post.id) ?? 0;
  if (count === 0) {
    return <p className="text-sm text-zinc-500">Be the first to like this</p>;
  }
  return <p className="text-sm font-semibold">{plural(count, "like")}</p>;
}

function IconButton({ label, onClick, className, children }: { label: string; onClick: () => void; className?: string; children: React.ReactNode }) {
  return (
    <button type="button" aria-label={label} title={label} onClick={onClick} className={cn("p-2 hover:opacity-60", className)}>
      {children}
    </button>
  );
}

/** The "…" menu: open the post, copy its link, and delete it when it is yours. */
export function PostMenu({ social, post }: { social: Social; post: Post }) {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const mine = post.authorId === social.me;
  const row = "w-full border-t py-3.5 text-center text-sm first:border-t-0 hover:bg-zinc-50";
  return (
    <>
      <button type="button" aria-label="More options" onClick={() => setOpen(true)} className="p-1 hover:opacity-60">
        <MoreHorizontal className="size-5" />
      </button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-sm gap-0 overflow-hidden p-0 [&>button:last-child]:hidden">
          <DialogTitle className="sr-only">Post options</DialogTitle>
          {mine ? (
            <button
              type="button"
              className={cn(row, "font-semibold text-red-600")}
              onClick={() => {
                setOpen(false);
                void db.delete("Post", post.id);
                if (window.location.pathname.startsWith("/p/")) router.push("/");
              }}
            >
              Delete
            </button>
          ) : null}
          <button type="button" className={row} onClick={() => { setOpen(false); router.push(`/p/${post.id}`); }}>
            Go to post
          </button>
          <button
            type="button"
            className={row}
            onClick={() => {
              setOpen(false);
              void navigator.clipboard?.writeText(`${window.location.origin}/p/${post.id}`);
            }}
          >
            Copy link
          </button>
          <button type="button" className={row} onClick={() => setOpen(false)}>
            Cancel
          </button>
        </DialogContent>
      </Dialog>
    </>
  );
}

/**
 * The photo. A double tap likes the post and pops a heart over it, the way
 * people expect a photo app to work.
 */
export function PostPhoto({ social, post, className, fit = "cover" }: { social: Social; post: Post; className?: string; fit?: "cover" | "contain" }) {
  const [pops, setPops] = useState(0);
  return (
    <div
      className={cn("relative select-none overflow-hidden bg-zinc-100", className)}
      onDoubleClick={() => {
        actions.like(social, post.id);
        setPops((n) => n + 1);
      }}
    >
      <img
        src={post.imageUrl}
        alt={post.caption ?? ""}
        loading="lazy"
        draggable={false}
        className={cn("size-full", fit === "cover" ? "object-cover" : "object-contain")}
      />
      {pops > 0 ? (
        <span key={pops} className="pointer-events-none absolute inset-0 grid place-items-center">
          <Heart className="heart-pop size-24 fill-white text-white drop-shadow-lg" strokeWidth={1} />
        </span>
      ) : null}
    </div>
  );
}
