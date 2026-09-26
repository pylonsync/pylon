"use client";

import React, { useMemo, useState } from "react";
import { siteConfig } from "@/lib/site.config";
import { groupConversations, type ConversationRow } from "@/lib/chat";
import { BrandMark, CloseIcon, ComposeIcon, PencilIcon, SearchIcon, SignOutIcon, TrashIcon } from "./icons";

export type Account = { kind: "guest" } | { kind: "user"; email: string | null };

export interface SidebarProps {
  conversations: ConversationRow[];
  currentId: string | null;
  now: number;
  account: Account;
  onSelect: (id: string) => void;
  onNew: () => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
  onSignIn: () => void;
  onSignOut: () => void;
  // Mobile drawer: shows a close button when set.
  onClose?: () => void;
}

export function Sidebar({
  conversations,
  currentId,
  now,
  account,
  onSelect,
  onNew,
  onRename,
  onDelete,
  onSignIn,
  onSignOut,
  onClose,
}: SidebarProps) {
  const [filter, setFilter] = useState("");
  const groups = useMemo(() => {
    const f = filter.trim().toLowerCase();
    const rows = f ? conversations.filter((c) => c.title.toLowerCase().includes(f)) : conversations;
    return groupConversations(rows, now);
  }, [conversations, filter, now]);

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-14 items-center justify-between px-3">
        <button
          type="button"
          onClick={onNew}
          className="press flex items-center gap-2.5 rounded-xl px-1.5 py-1 transition-colors duration-200 hover:bg-hover"
        >
          <BrandMark size={26} />
          <span className="text-[15.5px] font-semibold tracking-[-0.015em] text-ink">{siteConfig.brand.name}</span>
        </button>
        {onClose ? (
          <button
            type="button"
            onClick={onClose}
            aria-label="Close sidebar"
            className="press flex size-9 items-center justify-center rounded-xl text-ink-2 hover:bg-hover"
          >
            <CloseIcon size={18} />
          </button>
        ) : null}
      </div>

      <div className="space-y-1 px-3 pb-2">
        <button
          type="button"
          onClick={onNew}
          className="press flex h-10 w-full items-center gap-2.5 rounded-xl px-3 text-[14px] font-medium text-ink transition-colors duration-200 hover:bg-hover"
        >
          <ComposeIcon size={17} />
          New chat
        </button>
        <label className="flex h-10 items-center gap-2.5 rounded-xl px-3 text-ink-3 transition-colors duration-200 focus-within:bg-hover hover:bg-hover">
          <SearchIcon size={17} />
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Search chats"
            aria-label="Search chats"
            className="w-full bg-transparent text-[14px] text-ink outline-none placeholder:text-ink-3"
          />
        </label>
      </div>

      <nav className="sidebar-scroll min-h-0 flex-1 overflow-y-auto px-3 pb-4" aria-label="Conversations">
        {groups.length === 0 ? (
          <p className="px-3 py-6 text-[13px] leading-relaxed text-ink-3">
            {filter ? "No chats match that search." : "Your chats appear here."}
          </p>
        ) : (
          groups.map((g) => (
            <section key={g.label} className="mt-4 first:mt-2">
              <h3 className="px-3 pb-1 text-[12px] font-medium text-ink-3">{g.label}</h3>
              <ul className="space-y-px">
                {g.items.map((c) => (
                  <ConversationItem
                    key={c.id}
                    convo={c}
                    active={c.id === currentId}
                    onSelect={() => onSelect(c.id)}
                    onRename={(t) => onRename(c.id, t)}
                    onDelete={() => onDelete(c.id)}
                  />
                ))}
              </ul>
            </section>
          ))
        )}
      </nav>

      <AccountPanel account={account} onSignIn={onSignIn} onSignOut={onSignOut} />
    </div>
  );
}

function ConversationItem({
  convo,
  active,
  onSelect,
  onRename,
  onDelete,
}: {
  convo: ConversationRow;
  active: boolean;
  onSelect: () => void;
  onRename: (title: string) => void;
  onDelete: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(convo.title);
  const [confirming, setConfirming] = useState(false);

  function commit() {
    setEditing(false);
    const t = draft.trim();
    if (t && t !== convo.title) onRename(t);
    else setDraft(convo.title);
  }

  if (editing) {
    return (
      <li>
        <input
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") {
              setDraft(convo.title);
              setEditing(false);
            }
          }}
          aria-label="Chat title"
          className="h-9 w-full rounded-xl bg-surface px-3 text-[14px] text-ink outline-none ring-1 ring-brand/40"
        />
      </li>
    );
  }

  return (
    <li className="group/item relative">
      <button
        type="button"
        onClick={onSelect}
        aria-current={active ? "page" : undefined}
        title={convo.title}
        className={
          "flex h-9 w-full items-center gap-2 rounded-xl pl-3 pr-2 text-left text-[14px] transition-colors duration-200 " +
          (active ? "bg-active text-ink" : "text-ink-2 hover:bg-hover hover:text-ink")
        }
      >
        <span className="min-w-0 flex-1 truncate">{convo.title || "New chat"}</span>
        {convo.example ? (
          <span className="shrink-0 rounded-md bg-hover px-1.5 py-px text-[11px] text-ink-3 group-hover/item:hidden">
            Example
          </span>
        ) : null}
      </button>
      <div
        className={
          "absolute right-1 top-1/2 -translate-y-1/2 items-center gap-px rounded-lg pl-3 " +
          (confirming ? "flex" : "hidden group-focus-within/item:flex group-hover/item:flex") +
          (active ? " fade-active" : " fade-hover")
        }
      >
        {confirming ? (
          <>
            <button
              type="button"
              onClick={() => {
                setConfirming(false);
                onDelete();
              }}
              className="h-7 rounded-lg px-2 text-[12.5px] font-medium text-danger hover:bg-danger/10"
            >
              Delete
            </button>
            <button
              type="button"
              onClick={() => setConfirming(false)}
              className="h-7 rounded-lg px-2 text-[12.5px] text-ink-2 hover:bg-hover"
            >
              Cancel
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              onClick={() => {
                setDraft(convo.title);
                setEditing(true);
              }}
              aria-label="Rename chat"
              className="flex size-7 items-center justify-center rounded-lg text-ink-3 hover:bg-hover hover:text-ink"
            >
              <PencilIcon size={15} />
            </button>
            <button
              type="button"
              onClick={() => setConfirming(true)}
              aria-label="Delete chat"
              className="flex size-7 items-center justify-center rounded-lg text-ink-3 hover:bg-hover hover:text-danger"
            >
              <TrashIcon size={15} />
            </button>
          </>
        )}
      </div>
    </li>
  );
}

function AccountPanel({
  account,
  onSignIn,
  onSignOut,
}: {
  account: Account;
  onSignIn: () => void;
  onSignOut: () => void;
}) {
  if (account.kind === "guest") {
    return (
      <div className="border-t border-line p-3">
        <div className="rounded-2xl bg-surface/70 p-3.5 ring-1 ring-line">
          <p className="text-[13.5px] font-medium text-ink">You are using a guest session</p>
          <p className="mt-1 text-[12.5px] leading-relaxed text-ink-3">
            Chats are saved in this browser. Sign in to keep them on every device.
          </p>
          <button
            type="button"
            onClick={onSignIn}
            className="press mt-3 flex h-9 w-full items-center justify-center rounded-xl bg-ink text-[13.5px] font-medium text-bg transition-opacity duration-200 hover:opacity-90"
          >
            Sign in or create account
          </button>
        </div>
      </div>
    );
  }
  const email = account.email ?? "Signed in";
  return (
    <div className="border-t border-line p-3">
      <div className="flex items-center gap-3 rounded-xl px-2 py-1.5">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-brand-soft text-[13px] font-semibold uppercase text-brand">
          {email.slice(0, 1)}
        </span>
        <span className="min-w-0 flex-1 truncate text-[13.5px] text-ink">{email}</span>
        <button
          type="button"
          onClick={onSignOut}
          aria-label="Sign out"
          title="Sign out"
          className="press flex size-8 items-center justify-center rounded-lg text-ink-3 hover:bg-hover hover:text-ink"
        >
          <SignOutIcon size={17} />
        </button>
      </div>
    </div>
  );
}
