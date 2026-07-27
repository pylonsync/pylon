import React, { useState } from "react";
import { CalendarDays, Mail, PhoneCall, StickyNote } from "lucide-react";
import { cn } from "@/lib/utils";
import { Avatar } from "@/components/avatar";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { ACTIVITY_KINDS, relativeTime, type ActivityKind } from "@/lib/pipeline";

export interface ActivityRow {
  id: string;
  kind: string;
  body: string;
  ownerId?: string | null;
  createdAt?: string | null;
}

const ICON: Record<string, React.ReactNode> = {
  note: <StickyNote />,
  call: <PhoneCall />,
  email: <Mail />,
  meeting: <CalendarDays />,
};

/**
 * What happened on this deal, newest first, with the composer on top so logging
 * a call takes one click from landing on the page.
 */
export function ActivityTimeline({
  activities,
  ownerName,
  onLog,
}: {
  activities: ActivityRow[];
  ownerName: (id: string | null | undefined) => string | null;
  onLog: (kind: ActivityKind, body: string) => void | Promise<void>;
}) {
  const [kind, setKind] = useState<ActivityKind>("note");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);

  const ordered = [...activities].sort((a, b) =>
    (b.createdAt ?? "").localeCompare(a.createdAt ?? ""),
  );

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    const text = body.trim();
    if (!text || busy) return;
    setBusy(true);
    try {
      await onLog(kind, text);
      setBody("");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-4">
      <form onSubmit={submit} className="rounded-lg border border-border bg-card p-2.5">
        <Textarea
          value={body}
          rows={2}
          placeholder="Log a call, note, or next step…"
          aria-label="Activity"
          onChange={(event) => setBody(event.target.value)}
          onKeyDown={(event) => {
            // ⌘/Ctrl+Enter submits — the composer is small and always focused
            // when you're typing into it.
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              submit(event);
            }
          }}
          className="min-h-16 resize-none border-0 bg-transparent p-0 shadow-none focus-visible:ring-0"
        />
        <div className="mt-2 flex items-center justify-between gap-2">
          <div className="w-32">
            <Select
              aria-label="Activity type"
              value={kind}
              onChange={(event) => setKind(event.target.value as ActivityKind)}
              className="h-7 text-[12px]"
            >
              {ACTIVITY_KINDS.map((value) => (
                <option key={value} value={value}>
                  {value[0].toUpperCase() + value.slice(1)}
                </option>
              ))}
            </Select>
          </div>
          <Button type="submit" size="sm" disabled={!body.trim() || busy}>
            {busy ? "Saving…" : "Log"}
          </Button>
        </div>
      </form>

      {ordered.length === 0 ? (
        <p className="px-1 py-6 text-center text-[12px] text-muted-foreground">
          No activity yet. Log the first call or note above.
        </p>
      ) : (
        <ol className="space-y-3">
          {ordered.map((activity) => (
            <li key={activity.id} className="flex gap-2.5">
              <div
                className={cn(
                  "mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md",
                  "bg-surface-2 text-muted-foreground [&_svg]:size-3",
                )}
              >
                {ICON[activity.kind] ?? ICON.note}
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                  <Avatar name={ownerName(activity.ownerId) ?? "?"} size="sm" />
                  <span className="capitalize">{activity.kind}</span>
                  <span aria-hidden="true">·</span>
                  <time dateTime={activity.createdAt ?? undefined}>
                    {relativeTime(activity.createdAt)}
                  </time>
                </div>
                <p className="mt-1 whitespace-pre-wrap text-[13px] leading-5">
                  {activity.body}
                </p>
              </div>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
