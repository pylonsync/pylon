"use client";

import React, { useEffect, useRef, useState } from "react";
import { db } from "@pylonsync/react";
import { EnsureGuest, useAuth } from "@pylonsync/client";
import { Button } from "@/components/ui/button";

export interface Message {
  id: string;
  authorId: string;
  text: string;
  createdAt: string;
}

// `<EnsureGuest>` mints a guest session so anyone can chat with no login.
// Inside it, `useAuth()` exposes the guest's `userId` so we can right-align
// your own messages.
export function ChatRoom() {
  return (
    <EnsureGuest fallback={<div className="flex-1" />}>
      <Room />
    </EnsureGuest>
  );
}

function Room() {
  const { userId } = useAuth();
  const [text, setText] = useState("");
  const { data: messages } = db.useQuery<Message>("Message");
  const bottomRef = useRef<HTMLDivElement>(null);

  // Oldest → newest, like a chat transcript.
  const ordered = [...messages].sort((a, b) =>
    a.createdAt.localeCompare(b.createdAt),
  );

  // db.useQuery is live, so this fires whenever a message arrives (here or in
  // another tab) — keep the newest message in view.
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [ordered.length]);

  async function send(e: React.FormEvent) {
    e.preventDefault();
    const value = text.trim();
    if (!value) return;
    setText("");
    // No authorId sent — field.owner() stamps it from the session.
    await db.insert("Message", { text: value });
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto py-2">
        {ordered.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No messages yet — say hi. Open a second tab to watch it arrive live.
          </p>
        ) : (
          ordered.map((m) => {
            const mine = m.authorId === userId;
            return (
              <div
                key={m.id}
                className={mine ? "flex justify-end" : "flex justify-start"}
              >
                <div
                  className={
                    "max-w-[80%] rounded-2xl px-3 py-1.5 text-sm " +
                    (mine
                      ? "bg-primary text-primary-foreground"
                      : "bg-muted text-foreground")
                  }
                >
                  {!mine && (
                    <div className="mb-0.5 font-mono text-[10px] text-muted-foreground">
                      {shortId(m.authorId)}
                    </div>
                  )}
                  <span className="whitespace-pre-wrap break-words">
                    {m.text}
                  </span>
                </div>
              </div>
            );
          })
        )}
        <div ref={bottomRef} />
      </div>

      <form
        onSubmit={send}
        className="shrink-0 flex items-center gap-2 border-t py-3"
      >
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Message…"
          aria-label="Message"
          autoComplete="off"
          className="flex h-10 w-full rounded-full border border-input bg-transparent px-4 py-1 text-sm shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        <Button type="submit" disabled={!text.trim()}>
          Send
        </Button>
      </form>
    </div>
  );
}

function shortId(id: string) {
  return "@" + id.replace(/^guest_/, "").slice(0, 6);
}
