import React from "react";
import { Search } from "lucide-react";
import { Avatar } from "@/components/avatar";
import { formatHandle } from "@/lib/handles";
import { listTime } from "@/lib/format";
import { cn } from "@/lib/utils";

export interface ConversationSummary {
  id: string;
  name: string | null;
  handle: string;
  preview: string;
  lastMessageAt: string | null;
  status: "allowed" | "blocked" | "unknown";
  thinking: boolean;
  needsAttention: boolean;
  demo: boolean;
}

export function ConversationList({
  conversations,
  selectedId,
  query,
  onQuery,
  onSelect,
}: {
  conversations: ConversationSummary[];
  selectedId: string | null;
  query: string;
  onQuery: (q: string) => void;
  onSelect: (id: string) => void;
}) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="px-3 pb-2 pt-1">
        <label className="flex h-8 items-center gap-2 rounded-lg bg-black/[0.045] px-2.5 text-[13px] text-muted-foreground transition-colors focus-within:bg-black/[0.06] dark:bg-white/[0.06]">
          <Search className="size-3.5" strokeWidth={1.75} />
          <input
            value={query}
            onChange={(e) => onQuery(e.target.value)}
            placeholder="Search"
            aria-label="Search conversations"
            className="w-full bg-transparent text-foreground outline-none placeholder:text-muted-foreground"
          />
        </label>
      </div>
      <ul className="min-h-0 flex-1 overflow-y-auto px-2 pb-3" role="listbox" aria-label="Conversations">
        {conversations.length === 0 ? (
          <li className="px-3 py-10 text-center text-[13px] text-muted-foreground">
            {query ? "No matches." : "No conversations yet. Texts appear here as they arrive."}
          </li>
        ) : null}
        {conversations.map((c) => {
          const selected = c.id === selectedId;
          return (
            <li key={c.id}>
              <button
                type="button"
                role="option"
                aria-selected={selected}
                onClick={() => onSelect(c.id)}
                className={cn(
                  "group flex w-full items-center gap-3 rounded-[0.8rem] px-2.5 py-2 text-left transition-[background-color] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)]",
                  selected ? "bg-bubble-out text-white" : "hover:bg-black/[0.04] dark:hover:bg-white/[0.05]",
                )}
              >
                <Avatar name={c.name} handle={c.handle} size={40} />
                <span className="min-w-0 flex-1">
                  <span className="flex items-baseline justify-between gap-2">
                    <span className="truncate text-[14px] font-semibold tracking-[-0.01em]">
                      {c.name ?? formatHandle(c.handle)}
                    </span>
                    <span
                      className={cn(
                        "shrink-0 text-[12px] tabular-nums",
                        selected ? "text-white/80" : "text-muted-foreground",
                      )}
                    >
                      {listTime(c.lastMessageAt)}
                    </span>
                  </span>
                  <span
                    className={cn(
                      "mt-0.5 line-clamp-2 text-[13px] leading-[1.3]",
                      selected ? "text-white/85" : "text-muted-foreground",
                    )}
                  >
                    {c.thinking ? <em className="not-italic">Writing a reply…</em> : c.preview}
                  </span>
                  <span className="mt-1 flex gap-1.5 empty:hidden">
                    {c.status === "unknown" ? <Tag selected={selected}>Not on allowlist</Tag> : null}
                    {c.status === "blocked" ? <Tag selected={selected}>Blocked</Tag> : null}
                    {c.needsAttention ? <Tag selected={selected} tone="warn">Unanswered</Tag> : null}
                    {c.demo ? <Tag selected={selected}>Sample</Tag> : null}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function Tag({
  children,
  selected,
  tone = "muted",
}: {
  children: React.ReactNode;
  selected: boolean;
  tone?: "muted" | "warn";
}) {
  return (
    <span
      className={cn(
        "rounded-full px-1.5 py-px text-[11px] font-medium",
        selected
          ? "bg-white/20 text-white"
          : tone === "warn"
            ? "bg-warn/12 text-warn"
            : "bg-black/[0.05] text-muted-foreground dark:bg-white/[0.08]",
      )}
    >
      {children}
    </span>
  );
}
