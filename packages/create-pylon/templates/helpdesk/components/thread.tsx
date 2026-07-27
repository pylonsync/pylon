import React, { useState } from "react";
import { Lock } from "lucide-react";
import { cn } from "@/lib/utils";
import { Avatar } from "@/components/avatar";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { relativeTime } from "@/lib/format";

export interface MessageRow {
  id: string;
  body: string;
  fromCustomer?: boolean | null;
  internal?: boolean | null;
  authorId?: string | null;
  createdAt?: string | null;
}

/**
 * The conversation, oldest first, with the reply box pinned at the bottom.
 *
 * Internal notes are visually distinct because sending one to the customer by
 * mistake is the expensive error in a helpdesk — the composer says which mode
 * it's in, and the note keeps that marking in the thread forever.
 */
export function Thread({
  messages,
  authorName,
  customerName,
  now,
  onSend,
}: {
  messages: MessageRow[];
  authorName: (id: string | null | undefined) => string | null;
  customerName: string | null;
  now?: number;
  onSend: (body: string, internal: boolean) => void | Promise<void>;
}) {
  const [body, setBody] = useState("");
  const [internal, setInternal] = useState(false);
  const [busy, setBusy] = useState(false);

  const ordered = [...messages].sort((a, b) =>
    (a.createdAt ?? "").localeCompare(b.createdAt ?? ""),
  );

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    const text = body.trim();
    if (!text || busy) return;
    setBusy(true);
    try {
      await onSend(text, internal);
      setBody("");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-6 py-5">
        {ordered.length === 0 ? (
          <p className="py-8 text-center text-[12px] text-muted-foreground">
            No messages on this ticket yet.
          </p>
        ) : (
          ordered.map((message) => {
            const who = message.fromCustomer
              ? customerName
              : authorName(message.authorId);
            return (
              <article
                key={message.id}
                className={cn(
                  "rounded-lg border p-3",
                  message.internal
                    ? "border-stage-proposal/30 bg-stage-proposal/5"
                    : message.fromCustomer
                      ? "border-border bg-surface-1"
                      : "border-border bg-card",
                )}
              >
                <div className="mb-1.5 flex items-center gap-2 text-[11px] text-muted-foreground">
                  <Avatar name={who ?? "?"} size="sm" />
                  <span className="font-medium text-foreground">{who ?? "Unknown"}</span>
                  {message.internal ? (
                    <span className="inline-flex items-center gap-1 text-stage-proposal">
                      <Lock className="size-3" />
                      Internal note
                    </span>
                  ) : null}
                  <time
                    className="ml-auto"
                    dateTime={message.createdAt ?? undefined}
                  >
                    {relativeTime(message.createdAt, now)}
                  </time>
                </div>
                <p className="whitespace-pre-wrap text-[13px] leading-6">
                  {message.body}
                </p>
              </article>
            );
          })
        )}
      </div>

      <form
        onSubmit={submit}
        className={cn(
          "shrink-0 border-t p-3 transition-colors",
          internal ? "border-stage-proposal/40 bg-stage-proposal/5" : "border-border",
        )}
      >
        <Textarea
          value={body}
          rows={3}
          placeholder={internal ? "Internal note — the customer won't see this…" : "Reply to the customer…"}
          aria-label={internal ? "Internal note" : "Reply"}
          onChange={(event) => setBody(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) submit(event);
          }}
          className="min-h-20 resize-none"
        />
        <div className="mt-2 flex items-center justify-between gap-2">
          <label className="flex items-center gap-2 text-[12px] text-muted-foreground">
            <input
              type="checkbox"
              checked={internal}
              onChange={(event) => setInternal(event.target.checked)}
              className="size-3.5 accent-[var(--stage-proposal)]"
            />
            Internal note
          </label>
          <Button type="submit" size="sm" disabled={!body.trim() || busy}>
            {busy ? "Sending…" : internal ? "Add note" : "Send reply"}
          </Button>
        </div>
      </form>
    </div>
  );
}
