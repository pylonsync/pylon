"use client";

import React, { useState } from "react";
import { Check, Hash, Pencil, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { MAX_NAME_LENGTH, type ChatMember } from "@/lib/chat";
import { Avatar } from "./avatar";

export interface SidebarChannel {
  id: string;
  slug: string;
  name: string;
  unread: number;
}

interface SidebarProps {
  workspace: string;
  channels: SidebarChannel[];
  activeSlug: string;
  onSelect: (slug: string) => void;
  online: ChatMember[];
  self: ChatMember | undefined;
  onRename: (name: string) => Promise<void>;
}

export function Sidebar({
  workspace,
  channels,
  activeSlug,
  onSelect,
  online,
  self,
  onRename,
}: SidebarProps) {
  return (
    <nav
      aria-label="Workspace"
      className="flex h-full flex-col bg-sidebar text-sidebar-foreground [--avatar-ring:var(--sidebar)]"
    >
      <div className="flex h-14 shrink-0 items-center gap-2.5 px-4">
        <span
          aria-hidden
          className="inline-flex size-7 items-center justify-center rounded-lg bg-signal text-[13px] font-semibold text-signal-foreground"
        >
          {workspace.slice(0, 1).toUpperCase()}
        </span>
        <span className="truncate text-[15px] font-semibold tracking-tight">{workspace}</span>
      </div>

      <div className="scroll-area min-h-0 flex-1 overflow-y-auto px-2 pb-4">
        <h2 className="px-2 pt-3 pb-1.5 text-[12.5px] font-medium text-sidebar-muted">Channels</h2>
        <ul className="space-y-px">
          {channels.map((c) => {
            const active = c.slug === activeSlug;
            return (
              <li key={c.id}>
                <button
                  type="button"
                  onClick={() => onSelect(c.slug)}
                  aria-current={active ? "page" : undefined}
                  className={cn(
                    "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[14.5px] transition-colors duration-150",
                    active
                      ? "bg-sidebar-accent text-sidebar-accent-foreground"
                      : c.unread > 0
                        ? "font-semibold text-sidebar-foreground hover:bg-sidebar-accent/60"
                        : "text-sidebar-muted-strong hover:bg-sidebar-accent/60 hover:text-sidebar-foreground",
                  )}
                >
                  <Hash className="size-4 shrink-0 opacity-60" strokeWidth={1.75} aria-hidden />
                  <span className="flex-1 truncate">{c.name}</span>
                  {c.unread > 0 && !active ? (
                    <span className="min-w-5 rounded-full bg-signal px-1.5 text-center text-[11px] leading-5 font-semibold text-signal-foreground tabular-nums">
                      {c.unread > 99 ? "99+" : c.unread}
                      <span className="sr-only"> unread</span>
                    </span>
                  ) : null}
                </button>
              </li>
            );
          })}
        </ul>

        <h2 className="px-2 pt-6 pb-1.5 text-[12.5px] font-medium text-sidebar-muted">
          Online <span className="tabular-nums">{online.length}</span>
        </h2>
        <ul className="space-y-px">
          {online.map((m) => (
            <li key={m.userId} className="flex h-9 items-center gap-2.5 rounded-md px-2 text-[14px]">
              <Avatar name={m.name} hue={m.hue} size="sm" online />
              <span className="truncate text-sidebar-muted-strong">{m.name}</span>
              {m.userId === self?.userId ? (
                <span className="text-[12px] text-sidebar-muted">you</span>
              ) : null}
            </li>
          ))}
        </ul>
      </div>

      {self ? <SelfCard self={self} onRename={onRename} /> : null}
    </nav>
  );
}

function SelfCard({ self, onRename }: { self: ChatMember; onRename: (name: string) => Promise<void> }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(self.name);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await onRename(draft);
      setEditing(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save the name.");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="shrink-0 border-t border-sidebar-border p-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]">
      {editing ? (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
          className="flex items-center gap-1.5 rounded-lg bg-sidebar-accent p-1.5"
        >
          <label htmlFor="display-name" className="sr-only">
            Display name
          </label>
          <input
            id="display-name"
            autoFocus
            value={draft}
            maxLength={MAX_NAME_LENGTH}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setEditing(false);
            }}
            className="h-8 min-w-0 flex-1 rounded-md bg-sidebar px-2 text-[14px] text-sidebar-foreground outline-none ring-1 ring-sidebar-border focus:ring-sidebar-ring"
          />
          <button
            type="submit"
            disabled={saving}
            aria-label="Save name"
            className="inline-flex size-8 items-center justify-center rounded-md text-sidebar-foreground hover:bg-sidebar"
          >
            <Check className="size-4" strokeWidth={1.75} aria-hidden />
          </button>
          <button
            type="button"
            onClick={() => setEditing(false)}
            aria-label="Cancel"
            className="inline-flex size-8 items-center justify-center rounded-md text-sidebar-muted hover:bg-sidebar"
          >
            <X className="size-4" strokeWidth={1.75} aria-hidden />
          </button>
        </form>
      ) : (
        <div className="flex items-center gap-2.5 rounded-lg p-1.5">
          <Avatar name={self.name} hue={self.hue} size="sm" online />
          <div className="min-w-0 flex-1 leading-tight">
            <div className="truncate text-[14px] font-medium">{self.name}</div>
            <div className="text-[12px] text-sidebar-muted">Guest</div>
          </div>
          <button
            type="button"
            onClick={() => {
              setDraft(self.name);
              setError(null);
              setEditing(true);
            }}
            aria-label="Edit display name"
            title="Edit display name"
            className="inline-flex size-8 items-center justify-center rounded-md text-sidebar-muted transition-colors hover:bg-sidebar-accent hover:text-sidebar-foreground"
          >
            <Pencil className="size-3.5" strokeWidth={1.75} aria-hidden />
          </button>
        </div>
      )}
      {error ? <p className="px-2 pt-1 text-[12px] text-red-300">{error}</p> : null}
    </div>
  );
}
