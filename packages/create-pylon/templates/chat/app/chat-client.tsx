"use client";

import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { db, useRoom, useSyncStatus } from "@pylonsync/react";
import { EnsureGuest, useAuth } from "@pylonsync/client";
import { Menu, X } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  guestHue,
  guestName,
  sortMessages,
  typingLabel,
  unreadCount,
  type ChatMember,
  type ChatMessage,
} from "@/lib/chat";
import { Avatar } from "@/components/chat/avatar";
import { Composer } from "@/components/chat/composer";
import { MessageList } from "@/components/chat/message-list";
import { Sidebar, type SidebarChannel } from "@/components/chat/sidebar";

interface Channel {
  id: string;
  slug: string;
  name: string;
  topic: string;
  position: number;
}

/** Presence data each tab publishes to the room. Other clients control it, so it only feeds comparisons. */
interface Presence {
  channelId?: unknown;
  typingAt?: unknown;
}

const WORKSPACE = "__APP_NAME__";
const PRESENCE_ROOM = "chat:lobby";
/** A typing flag older than this is ignored, so a closed tab does not type forever. */
const TYPING_TTL_MS = 6000;
/** Minimum gap between "still typing" presence updates. */
const TYPING_REFRESH_MS = 2000;

// `<EnsureGuest>` mints a guest session so anyone can chat with no sign-up.
// The server renders the skeleton; the live workspace mounts in the browser.
export function ChatRoom({ initialChannel }: { initialChannel?: string }) {
  return (
    <EnsureGuest fallback={<ChatSkeleton />}>
      <Workspace initialChannel={initialChannel} />
    </EnsureGuest>
  );
}

/** How long to wait for a guest session before offering a retry. */
const SESSION_TIMEOUT_MS = 10_000;

function Workspace({ initialChannel }: { initialChannel?: string }) {
  const { userId } = useAuth();
  const [stalled, setStalled] = useState(false);
  useEffect(() => {
    if (userId) return;
    const t = setTimeout(() => setStalled(true), SESSION_TIMEOUT_MS);
    return () => clearTimeout(t);
  }, [userId]);
  // The session resolves a moment after the guest token lands.
  if (!userId) {
    return (
      <ChatSkeleton
        message={stalled ? "Could not start a session. Check your connection." : null}
        onRetry={stalled ? () => window.location.reload() : undefined}
      />
    );
  }
  return <Chat selfId={userId} initialChannel={initialChannel} />;
}

function Chat({ selfId, initialChannel }: { selfId: string; initialChannel?: string }) {
  const { data: channelRows } = db.useQuery<Channel>("Channel");
  const { data: memberRows } = db.useQuery<ChatMember>("Member");
  const { data: messageRows } = db.useQuery<ChatMessage>("Message");
  const status = useSyncStatus();

  const [activeSlug, setActiveSlug] = useState(initialChannel || "general");
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [joinError, setJoinError] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const hidden = usePageHidden();
  const isDesktop = useMediaQuery("(min-width: 768px)");

  useEffect(() => {
    if (!drawerOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setDrawerOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [drawerOpen]);

  // Seeds the demo workspace on first boot and creates this guest's Member row.
  const [joinedAt, setJoinedAt] = useState<string | null>(null);
  useEffect(() => {
    db.fn<{ now: string }>("joinChat", {})
      .then((r) => setJoinedAt(r.now))
      .catch((e: unknown) => {
        setJoinError(e instanceof Error ? e.message : "Could not join the chat.");
      });
  }, [selfId]);

  const channels = useMemo(
    () => [...channelRows].sort((a, b) => a.position - b.position),
    [channelRows],
  );
  const active = channels.find((c) => c.slug === activeSlug) ?? channels[0];

  const members = useMemo(() => {
    const map = new Map<string, ChatMember>();
    for (const m of memberRows) map.set(m.userId, m);
    return map;
  }, [memberRows]);
  const self = members.get(selfId);

  const channelMessages = useMemo(
    () => (active ? sortMessages(messageRows.filter((m) => m.channelId === active.id)) : []),
    [messageRows, active],
  );

  // ---- presence: who is online, who is typing ----
  const room = useRoom(PRESENCE_ROOM, selfId, {
    initialPresence: { channelId: active?.id ?? null, typingAt: 0 },
  });
  const { setPresence } = room;
  const lastTypingSentRef = useRef(0);

  useEffect(() => {
    if (!active) return;
    lastTypingSentRef.current = 0;
    setPresence({ channelId: active.id, typingAt: 0 });
  }, [active?.id, setPresence]); // eslint-disable-line react-hooks/exhaustive-deps

  const onTyping = useCallback(() => {
    if (!active) return;
    const now = Date.now();
    if (now - lastTypingSentRef.current < TYPING_REFRESH_MS) return;
    lastTypingSentRef.current = now;
    setPresence({ channelId: active.id, typingAt: now });
  }, [active, setPresence]);

  const onStopTyping = useCallback(() => {
    if (!active || lastTypingSentRef.current === 0) return;
    lastTypingSentRef.current = 0;
    setPresence({ channelId: active.id, typingAt: 0 });
  }, [active, setPresence]);

  const memberFor = useCallback(
    (userId: string): ChatMember =>
      members.get(userId) ?? { id: userId, userId, name: guestName(userId), hue: guestHue(userId) },
    [members],
  );

  const online = useMemo(() => {
    const seen = new Set<string>([selfId]);
    const list: ChatMember[] = [memberFor(selfId)];
    for (const p of room.peers) {
      if (seen.has(p.user_id)) continue;
      seen.add(p.user_id);
      list.push(memberFor(p.user_id));
    }
    return list;
  }, [room.peers, selfId, memberFor]);

  // Typing flags expire on this browser's clock: each new `typingAt` value a
  // peer publishes is stamped with the time it arrived here. A peer's own
  // clock (skewed, or set to a future time) never keeps a flag alive.
  const [typingReceived, setTypingReceived] = useState<Record<string, number>>({});
  useEffect(() => {
    setTypingReceived((prev) => {
      const next: Record<string, number> = {};
      for (const p of room.peers) {
        const data = (p.data ?? {}) as Presence;
        if (p.user_id === selfId || data.channelId !== active?.id) continue;
        if (typeof data.typingAt !== "number" || data.typingAt <= 0) continue;
        const key = `${p.user_id}\u0000${data.typingAt}`;
        next[key] = prev[key] ?? Date.now();
      }
      return next;
    });
  }, [room.peers, selfId, active?.id]);

  // Re-render once a second while a flag is live, so it disappears on time.
  const [now, setNow] = useState(() => Date.now());
  const hasTyping = Object.keys(typingReceived).length > 0;
  useEffect(() => {
    if (!hasTyping) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [hasTyping]);

  const typingIds = new Set<string>();
  for (const [key, receivedAt] of Object.entries(typingReceived)) {
    if (Math.max(now, receivedAt) - receivedAt < TYPING_TTL_MS) typingIds.add(key.split("\u0000")[0]);
  }
  const typingNames = [...typingIds].map((id) => memberFor(id).name);

  // ---- unread marks, kept per browser in localStorage ----
  const lastSeen = useLastSeen(selfId, channels, joinedAt);
  const { mark } = lastSeen;
  const newestInActive = channelMessages[channelMessages.length - 1]?.createdAt;
  useEffect(() => {
    if (!active || !newestInActive) return;
    const markIfVisible = () => {
      if (document.visibilityState === "visible") mark(active.id, newestInActive);
    };
    markIfVisible();
    document.addEventListener("visibilitychange", markIfVisible);
    return () => document.removeEventListener("visibilitychange", markIfVisible);
  }, [active, newestInActive, mark]);

  const sidebarChannels: SidebarChannel[] = channels.map((c) => ({
    id: c.id,
    slug: c.slug,
    name: c.name,
    unread: unreadCount(messageRows, c.id, lastSeen.map[c.id], selfId),
  }));
  // The open channel counts too while the tab is in the background, since its
  // mark only advances while the tab is visible.
  const totalUnread = sidebarChannels.reduce(
    (n, c) => n + (c.id === active?.id && !hidden ? 0 : c.unread),
    0,
  );
  useEffect(() => {
    document.title = totalUnread > 0 ? `(${totalUnread}) ${WORKSPACE}` : WORKSPACE;
  }, [totalUnread]);

  function selectChannel(slug: string) {
    setActiveSlug(slug);
    setDrawerOpen(false);
    const url = slug === "general" ? window.location.pathname : `${window.location.pathname}?c=${encodeURIComponent(slug)}`;
    window.history.replaceState(window.history.state, "", url);
  }

  function send(text: string) {
    if (!active) return;
    setSendError(null);
    // authorId is the caller's own id so the optimistic row shows the right
    // author at once; field.owner() on the server rejects any other value.
    // createdAt is left out: the server stamps it.
    db.insert("Message", { channelId: active.id, authorId: selfId, text }).catch(() => {
      setSendError("Message not sent. Check your connection and try again.");
    });
  }

  async function rename(name: string) {
    await db.fn("renameMember", { name });
  }

  function remove(id: string) {
    if (window.confirm("Delete this message? Everyone in the channel will stop seeing it.")) {
      db.delete("Message", id).catch(console.warn);
    }
  }

  if (!active) return <ChatSkeleton message={joinError} />;

  return (
    <div className="flex h-dvh overflow-hidden bg-background">
      {/* Sidebar: a column on desktop, a drawer on phones. */}
      <div
        className={cn(
          "fixed inset-0 z-30 bg-black/40 transition-opacity duration-300 ease-out-soft md:hidden",
          drawerOpen ? "opacity-100" : "pointer-events-none opacity-0",
        )}
        onClick={() => setDrawerOpen(false)}
        aria-hidden
      />
      <aside
        // Closed on a phone, the drawer is off screen: keep it out of the tab
        // order and the accessibility tree.
        inert={!isDesktop && !drawerOpen}
        className={cn(
          "fixed inset-y-0 left-0 z-40 w-[284px] transition-transform duration-300 ease-out-soft md:static md:z-auto md:w-[264px] md:translate-x-0 md:transition-none",
          drawerOpen ? "translate-x-0 shadow-2xl" : "-translate-x-full",
        )}
      >
        <Sidebar
          workspace={WORKSPACE}
          channels={sidebarChannels}
          activeSlug={active.slug}
          onSelect={selectChannel}
          online={online}
          self={self}
          onRename={rename}
        />
        <button
          type="button"
          onClick={() => setDrawerOpen(false)}
          aria-label="Close channels"
          className={cn(
            "absolute top-3 -right-12 inline-flex size-9 items-center justify-center rounded-full bg-card text-foreground shadow-float md:hidden",
            drawerOpen ? "opacity-100" : "pointer-events-none opacity-0",
          )}
        >
          <X className="size-4" strokeWidth={1.75} aria-hidden />
        </button>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center gap-2 border-b border-border/70 bg-background/85 px-2 backdrop-blur-md sm:px-4">
          <button
            type="button"
            onClick={() => setDrawerOpen(true)}
            aria-label="Open channels"
            className="relative inline-flex size-9 items-center justify-center rounded-lg text-foreground hover:bg-hover md:hidden"
          >
            <Menu className="size-5" strokeWidth={1.75} aria-hidden />
            {totalUnread > 0 ? (
              <span aria-hidden className="absolute top-1.5 right-1.5 size-2 rounded-full bg-signal ring-2 ring-background" />
            ) : null}
          </button>
          <div className="min-w-0 flex-1 pl-1">
            <h1 className="flex items-center gap-1 text-[16px] font-semibold tracking-tight">
              <span className="text-muted-foreground">#</span>
              <span className="truncate">{active.name}</span>
            </h1>
            <p className="hidden truncate text-[12.5px] text-muted-foreground sm:block">{active.topic}</p>
          </div>
          {status === "reconnecting" || status === "offline" ? (
            <span className="shrink-0 rounded-full bg-amber-100 px-2.5 py-1 text-[12px] font-medium text-amber-900">
              Reconnecting
            </span>
          ) : null}
          <OnlineStack online={online} />
        </header>

        <MessageList
          key={active.id}
          channelName={active.name}
          channelTopic={active.topic}
          messages={channelMessages}
          members={members}
          selfId={selfId}
          onDelete={remove}
        />

        <div className="shrink-0 px-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] sm:px-5">
          <div className="max-w-4xl">
            <TypingIndicator names={typingNames} />
            <Composer
              channelName={active.name}
              channelKey={active.id}
              onSend={send}
              onTyping={onTyping}
              onStopTyping={onStopTyping}
            />
            <p className="h-5 px-1 pt-1 text-[11.5px] text-muted-foreground" aria-live="polite">
              {sendError ?? joinError ?? (
                <span className="hidden sm:inline">
                  <kbd className="font-sans font-medium">Enter</kbd> to send,{" "}
                  <kbd className="font-sans font-medium">Shift + Enter</kbd> for a new line
                </span>
              )}
            </p>
          </div>
        </div>
      </main>
    </div>
  );
}

function OnlineStack({ online }: { online: ChatMember[] }) {
  const shown = online.slice(0, 4);
  return (
    <div className="flex shrink-0 items-center gap-2 pl-2" aria-label={`${online.length} online`}>
      <div className="flex -space-x-1 [--avatar-ring:var(--background)]">
        {shown.map((m) => (
          <Avatar key={m.userId} name={m.name} hue={m.hue} size="sm" className="rounded-full ring-2 ring-background" />
        ))}
      </div>
      <span className="flex items-center gap-1.5 text-[13px] text-muted-foreground tabular-nums">
        <span aria-hidden className="size-1.5 rounded-full bg-online" />
        {online.length} online
      </span>
    </div>
  );
}

function TypingIndicator({ names }: { names: string[] }) {
  const label = typingLabel(names);
  return (
    <div className="flex h-6 items-center gap-2 px-1 text-[12.5px] text-muted-foreground" aria-live="polite">
      {label ? (
        <>
          <span aria-hidden className="typing-dots inline-flex items-center gap-[3px]">
            <span />
            <span />
            <span />
          </span>
          <span className="truncate">{label}</span>
        </>
      ) : null}
    </div>
  );
}

/**
 * Per-channel "last message seen" marks for unread badges. `startAt` is the
 * server time from joinChat: a channel without a mark starts there, so a first
 * visit shows no badges and later messages count.
 */
function useLastSeen(selfId: string, channels: Channel[], startAt: string | null) {
  const key = `chat:lastSeen:${selfId}`;
  const [map, setMap] = useState<Record<string, string>>(() => readMarks(key));

  useEffect(() => {
    if (!startAt) return;
    const missing = channels.filter((c) => !map[c.id]);
    if (missing.length === 0) return;
    setMap((prev) => {
      const next = { ...prev };
      for (const c of missing) next[c.id] ??= startAt;
      writeMarks(key, next);
      return next;
    });
  }, [channels, map, key, startAt]);

  const mark = useCallback(
    (channelId: string, iso: string) => {
      setMap((prev) => {
        if (prev[channelId] && prev[channelId] >= iso) return prev;
        const next = { ...prev, [channelId]: iso };
        writeMarks(key, next);
        return next;
      });
    },
    [key],
  );

  return { map, mark };
}

function readMarks(key: string): Record<string, string> {
  try {
    const raw = window.localStorage.getItem(key);
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    return parsed && typeof parsed === "object" ? (parsed as Record<string, string>) : {};
  } catch {
    return {};
  }
}

function writeMarks(key: string, marks: Record<string, string>) {
  try {
    window.localStorage.setItem(key, JSON.stringify(marks));
  } catch {
    // Storage can be full or blocked (private mode). Unread badges then reset per visit.
  }
}

function usePageHidden(): boolean {
  const [hidden, setHidden] = useState(() => document.visibilityState === "hidden");
  useEffect(() => {
    const update = () => setHidden(document.visibilityState === "hidden");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  return hidden;
}

function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const list = window.matchMedia(query);
    const update = () => setMatches(list.matches);
    update();
    list.addEventListener("change", update);
    return () => list.removeEventListener("change", update);
  }, [query]);
  return matches;
}

/** Layout-matched placeholder while the session and replica load. */
function ChatSkeleton({ message, onRetry }: { message?: string | null; onRetry?: () => void }) {
  return (
    <div className="flex h-dvh overflow-hidden bg-background" aria-busy="true">
      <div className="hidden w-[264px] shrink-0 flex-col gap-3 bg-sidebar p-4 md:flex">
        <div className="mb-4 h-7 w-32 rounded-lg bg-sidebar-accent" />
        <div className="h-4 w-20 rounded bg-sidebar-accent/70" />
        <div className="h-4 w-28 rounded bg-sidebar-accent/70" />
        <div className="h-4 w-24 rounded bg-sidebar-accent/70" />
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="h-14 shrink-0 border-b border-border/70" />
        <div className="flex w-full max-w-4xl flex-1 flex-col justify-end gap-5 px-6 pb-6">
          {message ? (
            <div className="flex flex-col items-center gap-3 text-center text-[14px] text-muted-foreground">
              <p>{message}</p>
              {onRetry ? (
                <button
                  type="button"
                  onClick={onRetry}
                  className="rounded-full bg-foreground px-4 py-1.5 text-[13px] font-medium text-background"
                >
                  Try again
                </button>
              ) : null}
            </div>
          ) : (
            [72, 48, 64].map((w, i) => (
              <div key={i} className="flex animate-pulse gap-3">
                <div className="size-9 rounded-full bg-muted" />
                <div className="flex-1 space-y-2 pt-1">
                  <div className="h-3 w-28 rounded bg-muted" />
                  <div className="h-3 rounded bg-muted" style={{ width: `${w}%` }} />
                </div>
              </div>
            ))
          )}
        </div>
        <div className="w-full max-w-4xl px-3 pb-8 sm:px-5">
          <div className="h-12 rounded-2xl bg-card shadow-composer" />
        </div>
      </div>
    </div>
  );
}
