"use client";

import React, { useState } from "react";
import { callFn } from "@pylonsync/react";
import { COMMENT_MAX } from "@/lib/social";
import { cn } from "@/lib/utils";

/** "Add a comment" with a Post button that appears once there is text. */
export function CommentForm({ postId, className, autoFocus }: { postId: string; className?: string; autoFocus?: boolean }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const value = text.trim();
    if (!value || busy) return;
    setBusy(true);
    setError(null);
    try {
      await callFn("addComment", { postId, text: value });
      setText("");
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not post that comment.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className={cn("flex items-center gap-2", className)}>
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        maxLength={COMMENT_MAX}
        placeholder={error ?? "Add a comment…"}
        aria-label="Add a comment"
        autoFocus={autoFocus}
        className={cn(
          "min-w-0 flex-1 bg-transparent py-2 text-sm outline-none placeholder:text-zinc-500",
          error ? "placeholder:text-red-600" : "",
        )}
      />
      {text.trim() ? (
        <button type="submit" disabled={busy} className="text-sm font-semibold text-brand hover:text-zinc-900 disabled:opacity-50">
          Post
        </button>
      ) : null}
    </form>
  );
}
