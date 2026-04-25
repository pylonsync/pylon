/**
 * Pylon chat demo.
 *
 * Exercises: auth sessions, live sync (messages), rooms (typing +
 * presence), reactions via toggleReaction, optimistic sends via
 * useMutation, threads, DMs. Two browser windows side-by-side give you
 * the full multiplayer experience.
 */

import React, { useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  db,
  useRoom,
  configureClient,
  storageKey,
} from "@pylonsync/react";
import {
  Loader2,
  Hash,
  Lock,
  Star,
  Plus,
  LogOut,
  Search,
  X,
  Send,
  MessageSquare,
  ChevronDown,
  Smile,
} from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { Label } from "@pylonsync/example-ui/label";
import { Card, CardContent } from "@pylonsync/example-ui/card";
import { Textarea } from "@pylonsync/example-ui/textarea";
import { useCollabText } from "@pylonsync/loro";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@pylonsync/example-ui/dialog";
import { Avatar, AvatarFallback } from "@pylonsync/example-ui/avatar";
import { Checkbox } from "@pylonsync/example-ui/checkbox";
import { Badge } from "@pylonsync/example-ui/badge";
import { cn } from "@pylonsync/example-ui/utils";

const BASE_URL = "http://localhost:4321";
// Give this app its own namespace so chat's auth + replica don't clobber
// any other Pylon app served from the same browser origin.
init({ baseUrl: BASE_URL, appName: "chat" });
configureClient({ baseUrl: BASE_URL, appName: "chat" });

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type Channel = {
  id: string;
  name: string;
  topic?: string;
  isPrivate: boolean | number;
  createdBy: string;
};

type Message = {
  id: string;
  channelId: string;
  authorId: string;
  body: string;
  parentMessageId?: string | null;
  editedAt?: string | null;
  createdAt: string;
};

type User = {
  id: string;
  email: string;
  displayName: string;
  avatarColor: string;
};

type Reaction = {
  id: string;
  messageId: string;
  userId: string;
  emoji: string;
};

type Membership = {
  id: string;
  channelId: string;
  userId: string;
  role: string;
  joinedAt: string;
};

type ReadMarker = {
  id: string;
  userId: string;
  channelId: string;
  lastReadAt: string;
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function initials(name: string | undefined): string {
  if (!name) return "·";
  const parts = name.trim().split(/\s+/);
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

function formatTime(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}

function sameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

function formatDateHeading(iso: string): string {
  const d = new Date(iso);
  const now = new Date();
  const yesterday = new Date();
  yesterday.setDate(now.getDate() - 1);
  if (sameDay(d, now)) return "Today";
  if (sameDay(d, yesterday)) return "Yesterday";
  const thisYear = d.getFullYear() === now.getFullYear();
  return d.toLocaleDateString(undefined, {
    weekday: "long",
    month: "short",
    day: "numeric",
    year: thisYear ? undefined : "numeric",
  });
}

function isDmChannel(ch: { name: string }): boolean {
  return ch.name.startsWith("dm:");
}

// ---------------------------------------------------------------------------
// Rich message body: autolinks + lightweight markdown (*bold*, _italic_,
// `inline code`, ```code blocks```). Deliberately tiny — a real chat would
// swap in a sanitizing parser. Runs a few regex passes + escapes the input
// so user content can't inject HTML.
// ---------------------------------------------------------------------------

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function renderMarkdown(body: string): string {
  const blocks: string[] = [];
  let work = body.replace(/```([\s\S]*?)```/g, (_, code: string) => {
    const idx = blocks.push(code) - 1;
    return `\u0000BLOCK${idx}\u0000`;
  });
  work = escapeHtml(work);
  work = work.replace(/`([^`]+)`/g, (_, c) => `<code>${c}</code>`);
  work = work.replace(/\*([^*\n]+)\*/g, "<strong>$1</strong>");
  work = work.replace(/(^|[^\w])_([^_\n]+)_(?=[^\w]|$)/g, "$1<em>$2</em>");
  work = work.replace(
    /(^|[\s(])((?:https?:\/\/)[^\s<>"']+)/g,
    (_, lead, url) =>
      `${lead}<a href="${url}" target="_blank" rel="noopener noreferrer" class="text-primary underline underline-offset-2 hover:text-primary/80">${url}</a>`,
  );
  work = work.replace(/\u0000BLOCK(\d+)\u0000/g, (_, idx) => {
    const raw = blocks[Number(idx)];
    return `<pre class="mt-1 overflow-x-auto rounded-md border border-border bg-muted px-3 py-2 text-xs"><code>${escapeHtml(raw.replace(/^\n/, ""))}</code></pre>`;
  });
  return work;
}

function RichBody({ body }: { body: string }) {
  return (
    <div
      className="whitespace-pre-wrap break-words text-sm leading-6 [&_code]:rounded [&_code]:bg-muted [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-[12.5px] [&_code]:text-foreground [&_strong]:font-semibold [&_em]:italic"
      dangerouslySetInnerHTML={{ __html: renderMarkdown(body) }}
    />
  );
}

function dmPeerId(ch: { name: string }, me: string): string | null {
  if (!isDmChannel(ch)) return null;
  const [, a, b] = ch.name.split(":");
  return a === me ? b : a === undefined ? null : a;
}

function starsKey(userId: string) {
  return storageKey(`stars:${userId}`);
}

function loadStars(userId: string): Set<string> {
  try {
    const raw = localStorage.getItem(starsKey(userId));
    return raw ? new Set(JSON.parse(raw) as string[]) : new Set();
  } catch {
    return new Set();
  }
}

function saveStars(userId: string, stars: Set<string>) {
  try {
    localStorage.setItem(starsKey(userId), JSON.stringify([...stars]));
  } catch {}
}

function useStars(userId: string): {
  stars: Set<string>;
  toggle: (id: string) => void;
} {
  const [stars, setStars] = useState<Set<string>>(() => loadStars(userId));
  const toggle = (id: string) => {
    setStars((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      saveStars(userId, next);
      return next;
    });
  };
  return { stars, toggle };
}

// ---------------------------------------------------------------------------
// ColorAvatar — reusable avatar that keeps per-user color tints.
// ---------------------------------------------------------------------------

function ColorAvatar({
  name,
  color,
  size = "md",
  onClick,
  className,
}: {
  name: string | undefined;
  color: string | undefined;
  size?: "xs" | "sm" | "md" | "lg";
  onClick?: () => void;
  className?: string;
}) {
  const sizeCls =
    size === "xs"
      ? "size-5 text-[9.5px]"
      : size === "sm"
        ? "size-6 text-[10.5px]"
        : size === "lg"
          ? "size-14 text-lg"
          : "size-8 text-xs";
  return (
    <Avatar
      className={cn(
        sizeCls,
        "font-semibold text-white shrink-0",
        onClick && "cursor-pointer",
        className,
      )}
      onClick={onClick}
      style={{ backgroundColor: color || "#8b5cf6" }}
    >
      <AvatarFallback
        className="bg-transparent text-white"
        style={{ backgroundColor: color || "#8b5cf6" }}
      >
        {initials(name)}
      </AvatarFallback>
    </Avatar>
  );
}

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

export function ChatApp() {
  const [currentUser, setCurrentUser] = useState<User | null>(() => {
    try {
      const token = localStorage.getItem(storageKey("token"));
      const cached = localStorage.getItem(storageKey("user"));
      return token && cached ? (JSON.parse(cached) as User) : null;
    } catch {
      return null;
    }
  });
  const [activeChannelId, setActiveChannelId] = useState<string | null>(null);
  const [threadMessageId, setThreadMessageId] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [dmPickerOpen, setDmPickerOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [profileUserId, setProfileUserId] = useState<string | null>(null);
  const [channelDetailsId, setChannelDetailsId] = useState<string | null>(null);

  const { data: liveUser } = db.useQueryOne<User>(
    "User",
    currentUser?.id ?? "",
  );
  useEffect(() => {
    if (liveUser && liveUser.id === currentUser?.id) {
      setCurrentUser(liveUser);
      localStorage.setItem(storageKey("user"), JSON.stringify(liveUser));
    }
  }, [liveUser, currentUser?.id]);

  useEffect(() => {
    const token = localStorage.getItem(storageKey("token"));
    if (!token || !currentUser) return;
    let cancelled = false;
    (async () => {
      try {
        const res = await fetch(`${BASE_URL}/api/auth/me`, {
          headers: { Authorization: `Bearer ${token}` },
        });
        if (!res.ok) return;
        const me = (await res.json()) as { user_id: string | null };
        if (cancelled) return;
        if (!me.user_id || me.user_id !== currentUser.id) {
          localStorage.removeItem(storageKey("token"));
          localStorage.removeItem(storageKey("user"));
          try {
            indexedDB.deleteDatabase(`pylon_sync_chat`);
          } catch {}
          setCurrentUser(null);
        }
      } catch {}
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    setThreadMessageId(null);
  }, [activeChannelId]);

  useEffect(() => {
    if (!currentUser) return;
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && !e.shiftKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen(true);
        return;
      }
      if (mod && e.shiftKey && e.key.toLowerCase() === "d") {
        e.preventDefault();
        setDmPickerOpen(true);
        return;
      }
      if (mod && e.key === "/") {
        e.preventDefault();
        setShortcutsOpen((v) => !v);
        return;
      }
      if (e.key === "Escape") {
        if (paletteOpen) return setPaletteOpen(false);
        if (dmPickerOpen) return setDmPickerOpen(false);
        if (shortcutsOpen) return setShortcutsOpen(false);
        if (threadMessageId) return setThreadMessageId(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [currentUser, paletteOpen, dmPickerOpen, shortcutsOpen, threadMessageId]);

  async function signOut() {
    const token = localStorage.getItem(storageKey("token"));
    localStorage.removeItem(storageKey("token"));
    localStorage.removeItem(storageKey("user"));
    if (token) {
      fetch(`${BASE_URL}/api/auth/session`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      }).catch(() => {});
    }
    try {
      indexedDB.deleteDatabase(`pylon_sync_chat`);
    } catch {}
    setCurrentUser(null);
    setActiveChannelId(null);
  }

  if (!currentUser) return <Login onReady={setCurrentUser} />;

  const ui = {
    openProfile: (id: string) => setProfileUserId(id),
    openChannelDetails: (id: string) => setChannelDetailsId(id),
  };

  return (
    <UIContext.Provider value={ui}>
      <div className="grid h-screen grid-cols-[260px_1fr] overflow-hidden bg-background text-foreground data-[thread=true]:grid-cols-[260px_1fr_380px]" data-thread={!!threadMessageId}>
        <Sidebar
          currentUser={currentUser}
          activeChannelId={activeChannelId}
          onSelectChannel={setActiveChannelId}
          onSignOut={signOut}
          dmPickerOpen={dmPickerOpen}
          setDmPickerOpen={setDmPickerOpen}
        />
        {activeChannelId ? (
          <ChannelView
            channelId={activeChannelId}
            currentUser={currentUser}
            threadMessageId={threadMessageId}
            onOpenThread={setThreadMessageId}
          />
        ) : (
          <main className="flex min-h-0 flex-col">
            <EmptyState
              title="Welcome to Pylon Chat"
              body="Pick a channel on the left or start a direct message."
            />
          </main>
        )}
        {threadMessageId && activeChannelId && (
          <ThreadPanel
            parentId={threadMessageId}
            channelId={activeChannelId}
            currentUser={currentUser}
            onClose={() => setThreadMessageId(null)}
          />
        )}
        {paletteOpen && (
          <CommandPalette
            currentUser={currentUser}
            onClose={() => setPaletteOpen(false)}
            onSelectChannel={(id) => {
              setActiveChannelId(id);
              setPaletteOpen(false);
            }}
          />
        )}
        {shortcutsOpen && (
          <ShortcutsHelp onClose={() => setShortcutsOpen(false)} />
        )}
        {profileUserId && (
          <ProfileModal
            userId={profileUserId}
            currentUser={currentUser}
            onClose={() => setProfileUserId(null)}
            onStartDm={(channelId) => {
              setProfileUserId(null);
              setActiveChannelId(channelId);
            }}
          />
        )}
        {channelDetailsId && (
          <ChannelDetailsModal
            channelId={channelDetailsId}
            currentUser={currentUser}
            onClose={() => setChannelDetailsId(null)}
            onOpenProfile={(id) => {
              setChannelDetailsId(null);
              setProfileUserId(id);
            }}
          />
        )}
      </div>
    </UIContext.Provider>
  );
}

const UIContext = React.createContext<{
  openProfile: (userId: string) => void;
  openChannelDetails: (channelId: string) => void;
}>({
  openProfile: () => {},
  openChannelDetails: () => {},
});

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

function Login({ onReady }: { onReady: (u: User) => void }) {
  const [mode, setMode] = useState<"signin" | "signup">("signin");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [name, setName] = useState("");
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function go() {
    setLoading(true);
    setErr(null);
    try {
      const path =
        mode === "signup"
          ? "/api/auth/password/register"
          : "/api/auth/password/login";
      const body =
        mode === "signup" ? { email, password, displayName: name } : { email, password };
      const res = await fetch(`${BASE_URL}${path}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const json = await res.json();
      if (!res.ok) {
        throw new Error(json?.error?.message ?? `HTTP ${res.status}`);
      }
      const token: string = json.token;
      localStorage.setItem(storageKey("token"), token);
      configureClient({ baseUrl: BASE_URL });
      // Server returns user_id; fetch the row for the cached User shape
      // the rest of the app expects in localStorage.
      const me = await fetch(`${BASE_URL}/api/entities/User/${json.user_id}`, {
        headers: { Authorization: `Bearer ${token}` },
      }).then((r) => r.json());
      localStorage.setItem(storageKey("user"), JSON.stringify(me));
      void db.sync.pull();
      onReady(me as User);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="grid min-h-screen place-items-center bg-gradient-to-br from-primary/15 via-background to-background p-6">
      <Card className="w-full max-w-sm">
        <CardContent className="p-7">
          <div className="mb-6 grid size-10 place-items-center rounded-lg bg-primary text-primary-foreground">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden>
              <path d="M4 7l8-4 8 4v10l-8 4-8-4V7z" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
              <path d="M12 11v10M4 7l8 4 8-4" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
            </svg>
          </div>
          <h1 className="text-xl font-semibold tracking-tight">
            {mode === "signup" ? "Create your account" : "Sign in to Pylon"}
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Local-first chat, powered by live sync.
          </p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void go();
            }}
            className="mt-5 flex flex-col gap-3"
          >
            <div className="grid gap-1.5">
              <Label htmlFor="login-email">Email</Label>
              <Input
                id="login-email"
                type="email"
                autoComplete="email"
                autoFocus
                required
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="you@example.com"
              />
            </div>
            {mode === "signup" && (
              <div className="grid gap-1.5">
                <Label htmlFor="login-name">Display name</Label>
                <Input
                  id="login-name"
                  autoComplete="name"
                  required
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="Alice"
                />
              </div>
            )}
            <div className="grid gap-1.5">
              <Label htmlFor="login-password">Password</Label>
              <Input
                id="login-password"
                type="password"
                autoComplete={mode === "signup" ? "new-password" : "current-password"}
                required
                minLength={8}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={mode === "signup" ? "At least 8 characters" : "Your password"}
              />
            </div>
            {err && (
              <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
                {err}
              </div>
            )}
            <Button type="submit" disabled={loading} className="mt-1">
              {loading && <Loader2 className="size-4 animate-spin" />}
              {loading
                ? mode === "signup"
                  ? "Creating account…"
                  : "Signing in…"
                : mode === "signup"
                  ? "Create account"
                  : "Sign in"}
            </Button>
            <button
              type="button"
              className="pt-1 text-center text-xs text-muted-foreground hover:text-foreground"
              onClick={() => {
                setErr(null);
                setMode((m) => (m === "signin" ? "signup" : "signin"));
              }}
            >
              {mode === "signin"
                ? "Don't have an account? Create one"
                : "Already have an account? Sign in"}
            </button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Unread counts
// ---------------------------------------------------------------------------

function useUnreadCounts(currentUser: User): Record<string, number> {
  const { data: messages } = db.useQuery<Message>("Message");
  const { data: markers } = db.useQuery<ReadMarker>("ReadMarker", {
    where: { userId: currentUser.id },
  });

  return useMemo(() => {
    const lastReadAt: Record<string, string> = {};
    for (const m of markers ?? []) lastReadAt[m.channelId] = m.lastReadAt;

    const counts: Record<string, number> = {};
    for (const msg of messages ?? []) {
      if (msg.authorId === currentUser.id) continue;
      const since = lastReadAt[msg.channelId];
      if (!since || msg.createdAt > since) {
        counts[msg.channelId] = (counts[msg.channelId] ?? 0) + 1;
      }
    }
    return counts;
  }, [messages, markers, currentUser.id]);
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

function Sidebar({
  currentUser,
  activeChannelId,
  onSelectChannel,
  onSignOut,
  dmPickerOpen,
  setDmPickerOpen,
}: {
  currentUser: User;
  activeChannelId: string | null;
  onSelectChannel: (id: string) => void;
  onSignOut: () => void;
  dmPickerOpen: boolean;
  setDmPickerOpen: (open: boolean) => void;
}) {
  const { data: channels } = db.useQuery<Channel>("Channel");
  const { data: myMemberships } = db.useQuery<Membership>("Membership", {
    where: { userId: currentUser.id },
  });
  const { stars, toggle: toggleStar } = useStars(currentUser.id);
  const unread = useUnreadCounts(currentUser);

  const [createModalOpen, setCreateModalOpen] = useState(false);

  const myChannelIds = useMemo(
    () => new Set((myMemberships ?? []).map((m) => m.channelId)),
    [myMemberships],
  );

  const { starred, regular, dms } = useMemo(() => {
    const s: Channel[] = [];
    const r: Channel[] = [];
    const d: Channel[] = [];
    for (const ch of channels ?? []) {
      if (isDmChannel(ch)) {
        if (myChannelIds.has(ch.id)) d.push(ch);
      } else if (stars.has(ch.id)) {
        s.push(ch);
      } else {
        r.push(ch);
      }
    }
    const byName = (a: Channel, b: Channel) => a.name.localeCompare(b.name);
    s.sort(byName);
    r.sort(byName);
    return { starred: s, regular: r, dms: d };
  }, [channels, stars, myChannelIds]);

  useEffect(() => {
    if (activeChannelId) return;
    const first = starred[0] || regular[0] || dms[0];
    if (first) onSelectChannel(first.id);
  }, [starred, regular, dms, activeChannelId, onSelectChannel]);

  const ui = React.useContext(UIContext);

  return (
    <>
      <aside className="flex min-h-0 flex-col border-r border-border bg-card/40">
        <div
          role="button"
          tabIndex={0}
          onClick={() => ui.openProfile(currentUser.id)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              ui.openProfile(currentUser.id);
            }
          }}
          className="flex shrink-0 cursor-pointer items-center gap-2.5 border-b border-border px-3 py-2.5 hover:bg-accent/40"
        >
          <div className="relative">
            <ColorAvatar name={currentUser.displayName} color={currentUser.avatarColor} />
            <span className="absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full bg-emerald-500 ring-2 ring-card" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-medium">{currentUser.displayName}</div>
            <div className="text-[11px] text-muted-foreground">Online</div>
          </div>
          <Button
            variant="ghost"
            size="icon"
            className="size-7 text-muted-foreground hover:text-foreground"
            onClick={(e) => {
              e.stopPropagation();
              onSignOut();
            }}
            title="Sign out"
            aria-label="Sign out"
          >
            <LogOut className="size-4" />
          </Button>
        </div>

        <nav className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-1.5 py-2">
          {starred.length > 0 && (
            <div>
              <SidebarSection label="Starred" />
              {starred.map((ch) => (
                <ChannelRow
                  key={ch.id}
                  channel={ch}
                  active={ch.id === activeChannelId}
                  unread={unread[ch.id] ?? 0}
                  starred
                  onSelect={() => onSelectChannel(ch.id)}
                  onToggleStar={() => toggleStar(ch.id)}
                />
              ))}
            </div>
          )}

          <div>
            <SidebarSection
              label={
                <>
                  Channels{" "}
                  <span className="font-normal text-muted-foreground/70">
                    {regular.length}
                  </span>
                </>
              }
              action={
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-5 text-muted-foreground hover:text-foreground"
                  onClick={() => setCreateModalOpen(true)}
                  title="Create channel"
                  aria-label="Create channel"
                >
                  <Plus className="size-3" />
                </Button>
              }
            />
            {regular.map((ch) => (
              <ChannelRow
                key={ch.id}
                channel={ch}
                active={ch.id === activeChannelId}
                unread={unread[ch.id] ?? 0}
                starred={false}
                onSelect={() => onSelectChannel(ch.id)}
                onToggleStar={() => toggleStar(ch.id)}
              />
            ))}
          </div>

          <div>
            <SidebarSection
              label="Direct Messages"
              action={
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-5 text-muted-foreground hover:text-foreground"
                  onClick={() => setDmPickerOpen(true)}
                  title="Start a DM"
                  aria-label="Start a DM"
                >
                  <Plus className="size-3" />
                </Button>
              }
            />
            {dms.map((ch) => (
              <DmRow
                key={ch.id}
                channel={ch}
                currentUser={currentUser}
                active={ch.id === activeChannelId}
                unread={unread[ch.id] ?? 0}
                onSelect={() => onSelectChannel(ch.id)}
              />
            ))}
            {dms.length === 0 && (
              <div className="px-3 pb-2 pt-0.5 text-xs text-muted-foreground">
                No DMs yet.
              </div>
            )}
          </div>
        </nav>
      </aside>
      {createModalOpen && (
        <CreateChannelModal
          onClose={() => setCreateModalOpen(false)}
          onCreated={(channelId) => {
            setCreateModalOpen(false);
            onSelectChannel(channelId);
          }}
        />
      )}
      {dmPickerOpen && (
        <DmPicker
          currentUser={currentUser}
          onClose={() => setDmPickerOpen(false)}
          onOpen={(channelId) => {
            setDmPickerOpen(false);
            onSelectChannel(channelId);
          }}
        />
      )}
    </>
  );
}

function SidebarSection({
  label,
  action,
}: {
  label: React.ReactNode;
  action?: React.ReactNode;
}) {
  return (
    <div className="mb-0.5 flex items-center justify-between px-2.5 pt-1 pb-1 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
      <span>{label}</span>
      {action}
    </div>
  );
}

function ChannelRow({
  channel,
  active,
  unread,
  starred,
  onSelect,
  onToggleStar,
}: {
  channel: Channel;
  active: boolean;
  unread: number;
  starred: boolean;
  onSelect: () => void;
  onToggleStar: () => void;
}) {
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      className={cn(
        "group flex cursor-pointer items-center gap-1.5 rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-accent/50 hover:text-foreground",
        active && "bg-primary/15 text-foreground hover:bg-primary/20",
        unread > 0 && !active && "font-semibold text-foreground",
      )}
    >
      <span className="flex size-4 items-center justify-center text-muted-foreground/70">
        {channel.isPrivate ? <Lock className="size-3" /> : <Hash className="size-3.5" />}
      </span>
      <span className="flex-1 truncate">{channel.name}</span>
      {unread > 0 && (
        <Badge variant="default" className="h-4 min-w-4 rounded-full px-1.5 text-[10px] leading-none">
          {unread}
        </Badge>
      )}
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onToggleStar();
        }}
        title={starred ? "Unstar" : "Star"}
        aria-label={starred ? "Unstar" : "Star"}
        className={cn(
          "flex size-5 items-center justify-center rounded opacity-0 transition-opacity hover:bg-accent group-hover:opacity-100",
          starred && "text-amber-400 opacity-100",
        )}
      >
        <Star className={cn("size-3", starred && "fill-current")} />
      </button>
    </div>
  );
}

function DmRow({
  channel,
  currentUser,
  active,
  unread,
  onSelect,
}: {
  channel: Channel;
  currentUser: User;
  active: boolean;
  unread: number;
  onSelect: () => void;
}) {
  const peerId = dmPeerId(channel, currentUser.id);
  const { data: peer } = db.useQueryOne<User>("User", peerId ?? "");
  const label = peer?.displayName ?? "Direct message";

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      className={cn(
        "flex cursor-pointer items-center gap-2 rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-accent/50 hover:text-foreground",
        active && "bg-primary/15 text-foreground hover:bg-primary/20",
        unread > 0 && !active && "font-semibold text-foreground",
      )}
    >
      <span className="size-2 rounded-full bg-emerald-500" />
      <span className="flex-1 truncate">{label}</span>
      {unread > 0 && (
        <Badge variant="default" className="h-4 min-w-4 rounded-full px-1.5 text-[10px] leading-none">
          {unread}
        </Badge>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Create-channel modal
// ---------------------------------------------------------------------------

function CreateChannelModal({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (channelId: string) => void;
}) {
  const [name, setName] = useState("");
  const [isPrivate, setIsPrivate] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const createChannel = db.useMutation<
    { name: string; topic: string; isPrivate: boolean },
    { channelId: string }
  >("createChannel");

  async function submit() {
    const cleaned = name.trim().toLowerCase();
    if (!cleaned) {
      setErr("channel name is required");
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      const res = await createChannel.mutate({
        name: cleaned,
        topic: "",
        isPrivate,
      });
      if (res?.channelId) onCreated(res.channelId);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (/UNIQUE constraint/i.test(msg)) {
        setErr(`#${cleaned} already exists`);
      } else if (/INVALID_NAME|lowercase/i.test(msg)) {
        setErr("Use lowercase letters, numbers, and dashes only.");
      } else {
        setErr(msg);
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Create a channel</DialogTitle>
          <DialogDescription>
            Channels are where conversations happen around a topic.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-4">
          <div className="grid gap-1.5">
            <Label htmlFor="channel-name">Name</Label>
            <Input
              id="channel-name"
              autoFocus
              value={name}
              onChange={(e) => {
                setName(e.target.value);
                if (err) setErr(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !busy) void submit();
              }}
              placeholder="team-chat"
            />
          </div>
          <label
            className="flex cursor-pointer items-start gap-3 rounded-lg border border-border bg-accent/30 px-3 py-2.5 hover:bg-accent/40"
          >
            <Checkbox
              checked={isPrivate}
              onCheckedChange={(v) => setIsPrivate(!!v)}
              className="mt-0.5"
            />
            <div>
              <div className="text-sm font-medium">
                {isPrivate ? "🔒 Private channel" : "# Public channel"}
              </div>
              <div className="mt-0.5 text-xs text-muted-foreground">
                {isPrivate
                  ? "Only invited members can see or join."
                  : "Anyone in the workspace can see and join."}
              </div>
            </div>
          </label>
          {err && (
            <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              {err}
            </div>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={() => void submit()}
            disabled={busy || name.trim().length === 0}
          >
            {busy && <Loader2 className="size-4 animate-spin" />}
            {busy ? "Creating…" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// DM picker
// ---------------------------------------------------------------------------

function DmPicker({
  currentUser,
  onClose,
  onOpen,
}: {
  currentUser: User;
  onClose: () => void;
  onOpen: (channelId: string) => void;
}) {
  const { data: allUsers } = db.useQuery<User>("User");
  const [query, setQuery] = useState("");
  const [opening, setOpening] = useState<string | null>(null);
  const startDm = db.useMutation<{ otherUserId: string }, { channelId: string }>(
    "startDm",
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (allUsers ?? [])
      .filter((u) => u.id !== currentUser.id)
      .filter(
        (u) =>
          !q ||
          u.displayName.toLowerCase().includes(q) ||
          u.email.toLowerCase().includes(q),
      )
      .sort((a, b) => a.displayName.localeCompare(b.displayName));
  }, [allUsers, query, currentUser.id]);

  async function open(u: User) {
    setOpening(u.id);
    try {
      const res = await startDm.mutate({ otherUserId: u.id });
      if (res?.channelId) onOpen(res.channelId);
    } catch (e) {
      console.error(e);
      setOpening(null);
    }
  }

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Start a direct message</DialogTitle>
          <DialogDescription>Pick someone to chat with.</DialogDescription>
        </DialogHeader>
        <Input
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search by name or email…"
        />
        <div className="-mx-6 max-h-[320px] overflow-y-auto px-2">
          {filtered.length === 0 ? (
            <div className="px-3 py-4 text-sm text-muted-foreground">
              No users match.
            </div>
          ) : (
            filtered.map((u) => (
              <button
                key={u.id}
                onClick={() => void open(u)}
                disabled={opening === u.id}
                className="flex w-full items-center gap-3 rounded-md px-3 py-2 text-left hover:bg-accent disabled:opacity-50"
              >
                <ColorAvatar name={u.displayName} color={u.avatarColor} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">
                    {u.displayName}
                  </div>
                  <div className="truncate text-xs text-muted-foreground">
                    {u.email}
                  </div>
                </div>
              </button>
            ))
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Channel view
// ---------------------------------------------------------------------------

function ChannelView({
  channelId,
  currentUser,
  threadMessageId,
  onOpenThread,
}: {
  channelId: string;
  currentUser: User;
  threadMessageId: string | null;
  onOpenThread: (id: string | null) => void;
}) {
  const { data: channel } = db.useQueryOne<Channel>("Channel", channelId);
  const markRead = db.useMutation<{ channelId: string }, unknown>(
    "markChannelRead",
  );

  useEffect(() => {
    if (channelId) void markRead.mutate({ channelId });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [channelId]);

  if (!channel) return <main className="flex min-h-0 flex-col" />;

  const isDm = isDmChannel(channel);
  const ui = React.useContext(UIContext);

  return (
    <main className="flex min-h-0 flex-col bg-background">
      <header className="flex shrink-0 items-center justify-between border-b border-border px-5 py-3">
        <div className="flex min-w-0 items-center gap-2">
          {isDm ? (
            <DmHeader channel={channel} currentUser={currentUser} />
          ) : (
            <>
              <button
                className="flex min-w-0 items-center gap-1.5 rounded-md px-1.5 py-0.5 text-[15px] font-semibold hover:bg-accent"
                onClick={() => ui.openChannelDetails(channel.id)}
                title="Channel details"
              >
                {channel.isPrivate ? (
                  <Lock className="size-3.5 text-muted-foreground" />
                ) : (
                  <Hash className="size-4 text-muted-foreground" />
                )}
                <span className="truncate">{channel.name}</span>
              </button>
              <span className="text-muted-foreground">·</span>
              <CollabTopic channelId={channel.id} />
            </>
          )}
        </div>
        <ChannelPresenceCount
          channelId={channelId}
          currentUser={currentUser}
        />
      </header>
      <MessageList
        channelId={channelId}
        currentUser={currentUser}
        onOpenThread={onOpenThread}
        threadMessageId={threadMessageId}
      />
      <Presence channelId={channelId} currentUser={currentUser} />
      <Composer channelId={channelId} currentUser={currentUser} />
    </main>
  );
}

/**
 * Inline-editable channel topic, backed by a Loro text CRDT.
 * Two browser tabs typing in the same channel header converge
 * character-by-character — concurrent edits to disjoint regions
 * both land instead of one stomping the other (which is what the
 * legacy LWW path would have done).
 *
 * Plays the role of the visible "this is real" demo for Pylon's
 * CRDT integration. The plumbing (server LoroStore, binary WS
 * broadcast, useLoroDoc hook, POST /api/crdt push endpoint) all
 * lights up the moment this component mounts and a user types.
 */
function CollabTopic({ channelId }: { channelId: string }) {
  const [value, setValue] = useCollabText("Channel", channelId, "topic");
  return (
    <input
      value={value}
      onChange={(e) => setValue(e.target.value)}
      placeholder="Set a topic — try editing this in two browser tabs"
      className="min-w-0 flex-1 truncate bg-transparent text-sm text-muted-foreground outline-none placeholder:text-muted-foreground/60 focus:text-foreground"
    />
  );
}

function DmHeader({
  channel,
  currentUser,
}: {
  channel: Channel;
  currentUser: User;
}) {
  const peerId = dmPeerId(channel, currentUser.id);
  const { data: peer } = db.useQueryOne<User>("User", peerId ?? "");
  const ui = React.useContext(UIContext);
  return (
    <button
      className="flex items-center gap-2 rounded-md px-1.5 py-0.5 text-[15px] font-semibold hover:bg-accent"
      onClick={() => peerId && ui.openProfile(peerId)}
      title="View profile"
    >
      <ColorAvatar
        name={peer?.displayName}
        color={peer?.avatarColor}
        size="sm"
      />
      <span>{peer?.displayName ?? "Direct message"}</span>
    </button>
  );
}

function ChannelPresenceCount({
  channelId,
  currentUser,
}: {
  channelId: string;
  currentUser: User;
}) {
  const { peers } = useRoom(`channel:${channelId}`, currentUser.id, {
    initialPresence: { displayName: currentUser.displayName },
  });
  const others = peers.filter((p) => p.user_id !== currentUser.id);
  const total = others.length + 1;
  const [open, setOpen] = useState(false);

  const wrapRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  return (
    <div className="relative" ref={wrapRef}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title="Who's here"
        className="flex items-center gap-1.5 rounded-md border border-border bg-card px-2 py-1 text-xs text-muted-foreground hover:text-foreground"
      >
        <span className="size-1.5 rounded-full bg-emerald-500" />
        <span>{others.length === 0 ? "Just you" : `${total} here`}</span>
      </button>
      {open && (
        <div className="absolute right-0 top-full z-20 mt-1 w-56 overflow-hidden rounded-md border border-border bg-popover text-popover-foreground shadow-md">
          <div className="border-b border-border px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
            In this channel
          </div>
          <PopoverPeerRow
            userId={currentUser.id}
            selfFallback={currentUser}
            isMe
          />
          {others.map((p) => (
            <PopoverPeerRow key={p.user_id} userId={p.user_id} isMe={false} />
          ))}
        </div>
      )}
    </div>
  );
}

function PopoverPeerRow({
  userId,
  selfFallback,
  isMe,
}: {
  userId: string;
  selfFallback?: User;
  isMe: boolean;
}) {
  const { data: user } = db.useQueryOne<User>("User", userId);
  const display = user ?? selfFallback;
  return (
    <div className="flex items-center gap-2 px-3 py-2">
      <ColorAvatar name={display?.displayName} color={display?.avatarColor} size="sm" />
      <div className="min-w-0 flex-1 truncate text-sm">
        {display?.displayName ?? "…"}
        {isMe && <span className="ml-1.5 text-muted-foreground">(you)</span>}
      </div>
      <span className="text-[10px] text-emerald-500">●</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

function MessageList({
  channelId,
  currentUser,
  onOpenThread,
  threadMessageId,
}: {
  channelId: string;
  currentUser: User;
  onOpenThread: (id: string | null) => void;
  threadMessageId: string | null;
}) {
  const { data: allMessages } = db.useQuery<Message>("Message", {
    where: { channelId },
    orderBy: { createdAt: "asc" },
  });
  const topLevel = useMemo(
    () =>
      (allMessages ?? []).filter(
        (m) => !m.parentMessageId || m.parentMessageId === null,
      ),
    [allMessages],
  );
  const replyCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const m of allMessages ?? []) {
      if (m.parentMessageId) {
        counts[m.parentMessageId] = (counts[m.parentMessageId] ?? 0) + 1;
      }
    }
    return counts;
  }, [allMessages]);

  const visible = useMemo(() => topLevel.slice(-100), [topLevel]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const [atBottom, setAtBottom] = useState(true);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onScroll = () => {
      const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
      setAtBottom(dist < 80);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    onScroll();
    return () => el.removeEventListener("scroll", onScroll);
  }, []);

  useEffect(() => {
    if (atBottom && scrollRef.current) {
      scrollRef.current.scrollTo({
        top: scrollRef.current.scrollHeight,
        behavior: "smooth",
      });
    }
  }, [visible.length, atBottom]);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    requestAnimationFrame(() => {
      el.scrollTo({ top: el.scrollHeight });
      setAtBottom(true);
    });
  }, [channelId]);

  if (visible.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto" ref={scrollRef}>
        <EmptyState
          title="No messages yet"
          body="Say something — the whole channel is listening."
        />
      </div>
    );
  }

  const rows: React.ReactNode[] = [];
  let prev: Message | undefined;
  for (const m of visible) {
    const prevDate = prev ? new Date(prev.createdAt) : null;
    const thisDate = new Date(m.createdAt);
    const needsDivider = !prev || !prevDate || !sameDay(prevDate, thisDate);
    if (needsDivider) {
      rows.push(
        <div
          key={`day-${m.id}`}
          className="my-3 flex items-center gap-3 px-5 text-[11px] uppercase tracking-wider text-muted-foreground"
        >
          <div className="h-px flex-1 bg-border" />
          <div className="rounded-full border border-border bg-card px-3 py-0.5 font-semibold">
            {formatDateHeading(m.createdAt)}
          </div>
          <div className="h-px flex-1 bg-border" />
        </div>,
      );
    }
    const compact =
      !!prev &&
      !needsDivider &&
      prev.authorId === m.authorId &&
      thisDate.getTime() - (prevDate?.getTime() ?? 0) < 5 * 60_000;
    rows.push(
      <MessageRow
        key={m.id}
        message={m}
        compact={compact}
        currentUser={currentUser}
        replyCount={replyCounts[m.id] ?? 0}
        threadOpen={threadMessageId === m.id}
        onOpenThread={() => onOpenThread(m.id)}
      />,
    );
    prev = m;
  }

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto pt-2 pb-1"
      >
        {rows}
      </div>
      <button
        onClick={() =>
          scrollRef.current?.scrollTo({
            top: scrollRef.current.scrollHeight,
            behavior: "smooth",
          })
        }
        aria-label="Jump to latest"
        className={cn(
          "pointer-events-none absolute bottom-3 left-1/2 flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-border bg-card px-3 py-1.5 text-xs font-medium text-foreground opacity-0 shadow-md transition-opacity",
          !atBottom && "pointer-events-auto opacity-100",
        )}
      >
        <ChevronDown className="size-3" />
        Jump to latest
      </button>
    </div>
  );
}

function MessageRow({
  message,
  compact,
  currentUser,
  replyCount,
  threadOpen,
  onOpenThread,
}: {
  message: Message;
  compact: boolean;
  currentUser: User;
  replyCount: number;
  threadOpen: boolean;
  onOpenThread: () => void;
}) {
  const { data: author } = db.useQueryOne<User>("User", message.authorId);
  const { data: reactions } = db.useQuery<Reaction>("Reaction", {
    where: { messageId: message.id },
  });
  const toggle = db.useMutation<
    { messageId: string; emoji: string },
    unknown
  >("toggleReaction");

  const grouped = useMemo(() => {
    const map: Record<string, { count: number; mine: boolean }> = {};
    for (const r of reactions ?? []) {
      if (!map[r.emoji]) map[r.emoji] = { count: 0, mine: false };
      map[r.emoji].count += 1;
      if (r.userId === currentUser.id) map[r.emoji].mine = true;
    }
    return Object.entries(map);
  }, [reactions, currentUser.id]);

  const ui = React.useContext(UIContext);
  const openAuthor = () => ui.openProfile(message.authorId);

  return (
    <div
      className={cn(
        "group relative flex items-start gap-3 px-5 py-0.5 hover:bg-accent/30",
        !compact && "mt-2 pt-1.5",
      )}
    >
      {compact ? (
        <div className="w-8 shrink-0 pt-1 text-right text-[10px] text-muted-foreground opacity-0 group-hover:opacity-100">
          {formatTime(message.createdAt)}
        </div>
      ) : (
        <div className="w-8 shrink-0">
          <ColorAvatar
            name={author?.displayName}
            color={author?.avatarColor}
            onClick={openAuthor}
          />
        </div>
      )}
      <div className="min-w-0 flex-1">
        {!compact && (
          <div className="mb-0.5 flex items-baseline gap-2">
            <button
              onClick={openAuthor}
              className="text-sm font-semibold hover:underline"
              title="View profile"
            >
              {author?.displayName ?? "…"}
            </button>
            <span className="text-[11px] text-muted-foreground">
              {formatTime(message.createdAt)}
            </span>
          </div>
        )}
        <RichBody body={message.body} />
        {message.editedAt && (
          <span className="ml-1 text-[11px] text-muted-foreground">
            (edited)
          </span>
        )}
        {grouped.length > 0 && (
          <div className="mt-1 flex flex-wrap gap-1">
            {grouped.map(([emoji, { count, mine }]) => (
              <button
                key={emoji}
                onClick={() =>
                  void toggle.mutate({ messageId: message.id, emoji })
                }
                className={cn(
                  "flex items-center gap-1 rounded-full border border-border bg-card px-1.5 py-0.5 text-xs hover:bg-accent",
                  mine && "border-primary/50 bg-primary/15 text-foreground",
                )}
              >
                <span>{emoji}</span>
                <span className="text-[11px] text-muted-foreground">
                  {count}
                </span>
              </button>
            ))}
          </div>
        )}
        {replyCount > 0 && (
          <button
            onClick={onOpenThread}
            className="mt-1 flex items-center gap-1.5 rounded-md border border-border bg-card px-2 py-1 text-xs font-medium text-primary hover:bg-accent"
            title="Open thread"
          >
            <MessageSquare className="size-3" />
            {replyCount} {replyCount === 1 ? "reply" : "replies"}
          </button>
        )}
      </div>
      <div
        className="absolute right-5 top-0 hidden items-center gap-0.5 rounded-md border border-border bg-card p-0.5 shadow-sm group-hover:flex"
        aria-hidden="true"
      >
        {["👍", "❤️", "🎉"].map((emoji) => (
          <button
            key={emoji}
            onClick={() =>
              void toggle.mutate({ messageId: message.id, emoji })
            }
            className="flex size-6 items-center justify-center rounded hover:bg-accent"
            title={`React ${emoji}`}
          >
            {emoji}
          </button>
        ))}
        <button
          onClick={onOpenThread}
          className={cn(
            "flex size-6 items-center justify-center rounded hover:bg-accent",
            threadOpen && "text-primary",
          )}
          title={threadOpen ? "Close thread" : "Reply in thread"}
        >
          <MessageSquare className="size-3.5" />
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Thread panel
// ---------------------------------------------------------------------------

function ThreadPanel({
  parentId,
  channelId,
  currentUser,
  onClose,
}: {
  parentId: string;
  channelId: string;
  currentUser: User;
  onClose: () => void;
}) {
  const { data: parent } = db.useQueryOne<Message>("Message", parentId);
  const { data: parentAuthor } = db.useQueryOne<User>(
    "User",
    parent?.authorId ?? "",
  );
  const { data: replies } = db.useQuery<Message>("Message", {
    where: { parentMessageId: parentId },
    orderBy: { createdAt: "asc" },
  });

  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    scrollRef.current?.scrollTo({
      top: scrollRef.current.scrollHeight,
      behavior: "smooth",
    });
  }, [(replies ?? []).length]);

  if (!parent) {
    return (
      <aside className="flex min-h-0 flex-col border-l border-border bg-card/40">
        <header className="flex shrink-0 items-center justify-between border-b border-border px-4 py-3">
          <div className="text-sm font-semibold">Thread</div>
          <Button variant="ghost" size="icon" className="size-7" onClick={onClose}>
            <X className="size-4" />
          </Button>
        </header>
        <div className="grid flex-1 place-items-center p-10 text-sm text-muted-foreground">
          Loading…
        </div>
      </aside>
    );
  }

  const replyList = replies ?? [];

  return (
    <aside className="flex min-h-0 flex-col border-l border-border bg-card/40">
      <header className="flex shrink-0 items-center justify-between border-b border-border px-4 py-3">
        <div>
          <div className="text-sm font-semibold">Thread</div>
          <div className="text-xs text-muted-foreground">
            {replyList.length} {replyList.length === 1 ? "reply" : "replies"}
          </div>
        </div>
        <Button variant="ghost" size="icon" className="size-7" onClick={onClose} aria-label="Close thread">
          <X className="size-4" />
        </Button>
      </header>
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-1 pb-1" ref={scrollRef}>
        <div className="border-b border-border px-4 py-3">
          <div className="flex items-start gap-3">
            <ColorAvatar name={parentAuthor?.displayName} color={parentAuthor?.avatarColor} />
            <div className="min-w-0 flex-1">
              <div className="mb-0.5 flex items-baseline gap-2">
                <span className="text-sm font-semibold">
                  {parentAuthor?.displayName ?? "…"}
                </span>
                <span className="text-[11px] text-muted-foreground">
                  {formatTime(parent.createdAt)}
                </span>
              </div>
              <RichBody body={parent.body} />
            </div>
          </div>
        </div>
        <div className="flex flex-col py-1">
          {replyList.map((m, i) => {
            const prev = replyList[i - 1];
            const compact =
              !!prev &&
              prev.authorId === m.authorId &&
              new Date(m.createdAt).getTime() -
                new Date(prev.createdAt).getTime() <
                5 * 60_000;
            return (
              <ThreadReplyRow
                key={m.id}
                message={m}
                compact={compact}
                currentUser={currentUser}
              />
            );
          })}
        </div>
      </div>
      <div className="shrink-0 border-t border-border p-3">
        <ThreadComposer
          parentId={parentId}
          channelId={channelId}
          currentUser={currentUser}
        />
      </div>
    </aside>
  );
}

function ThreadReplyRow({
  message,
  compact,
  currentUser,
}: {
  message: Message;
  compact: boolean;
  currentUser: User;
}) {
  const { data: author } = db.useQueryOne<User>("User", message.authorId);
  const { data: reactions } = db.useQuery<Reaction>("Reaction", {
    where: { messageId: message.id },
  });
  const toggle = db.useMutation<
    { messageId: string; emoji: string },
    unknown
  >("toggleReaction");

  const grouped = useMemo(() => {
    const map: Record<string, { count: number; mine: boolean }> = {};
    for (const r of reactions ?? []) {
      if (!map[r.emoji]) map[r.emoji] = { count: 0, mine: false };
      map[r.emoji].count += 1;
      if (r.userId === currentUser.id) map[r.emoji].mine = true;
    }
    return Object.entries(map);
  }, [reactions, currentUser.id]);

  return (
    <div className={cn("flex items-start gap-3 px-4 py-1", !compact && "mt-1.5")}>
      {compact ? (
        <div className="w-8 shrink-0 pt-1 text-right text-[10px] text-muted-foreground">
          {formatTime(message.createdAt)}
        </div>
      ) : (
        <div className="w-8 shrink-0">
          <ColorAvatar name={author?.displayName} color={author?.avatarColor} />
        </div>
      )}
      <div className="min-w-0 flex-1">
        {!compact && (
          <div className="mb-0.5 flex items-baseline gap-2">
            <span className="text-sm font-semibold">
              {author?.displayName ?? "…"}
            </span>
            <span className="text-[11px] text-muted-foreground">
              {formatTime(message.createdAt)}
            </span>
          </div>
        )}
        <RichBody body={message.body} />
        {grouped.length > 0 && (
          <div className="mt-1 flex flex-wrap gap-1">
            {grouped.map(([emoji, { count, mine }]) => (
              <button
                key={emoji}
                onClick={() =>
                  void toggle.mutate({ messageId: message.id, emoji })
                }
                className={cn(
                  "flex items-center gap-1 rounded-full border border-border bg-card px-1.5 py-0.5 text-xs hover:bg-accent",
                  mine && "border-primary/50 bg-primary/15 text-foreground",
                )}
              >
                <span>{emoji}</span>
                <span className="text-[11px] text-muted-foreground">{count}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function ThreadComposer({
  parentId,
  channelId,
  currentUser,
}: {
  parentId: string;
  channelId: string;
  currentUser: User;
}) {
  const [body, setBody] = useState("");
  const send = db.useMutation<
    { channelId: string; body: string; parentMessageId: string },
    unknown
  >("sendMessage");

  async function submit() {
    const text = body.trim();
    if (!text) return;
    setBody("");

    const tempId = `tmp_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    const store = db.sync.store;
    store.applyChange({
      seq: 0,
      entity: "Message",
      row_id: tempId,
      kind: "insert",
      data: {
        id: tempId,
        channelId,
        authorId: currentUser.id,
        parentMessageId: parentId,
        body: text,
        createdAt: new Date().toISOString(),
      },
      timestamp: "",
    });
    store.notify();

    try {
      await send.mutate({ channelId, body: text, parentMessageId: parentId });
    } catch (e) {
      console.error("reply failed", e);
    } finally {
      store.applyChange({
        seq: 0,
        entity: "Message",
        row_id: tempId,
        kind: "delete",
        timestamp: "",
      });
      store.notify();
    }
  }

  const canSend = body.trim().length > 0;
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 160) + "px";
  }, [body]);

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <ComposerInner
        textareaRef={textareaRef}
        value={body}
        onChange={setBody}
        placeholder="Reply to thread…"
        canSend={canSend}
        onSubmit={submit}
        ariaLabel="Send reply"
      />
    </form>
  );
}

// ---------------------------------------------------------------------------
// Typing indicator
// ---------------------------------------------------------------------------

function Presence({
  channelId,
  currentUser,
}: {
  channelId: string;
  currentUser: User;
}) {
  const { peers } = useRoom(`channel:${channelId}`, currentUser.id, {
    initialPresence: { displayName: currentUser.displayName, typing: false },
  });

  const typing = peers.filter(
    (p) =>
      p.user_id !== currentUser.id &&
      (p.data as { typing?: boolean })?.typing,
  );

  if (typing.length === 0) return <div className="h-5 shrink-0" />;

  const names = typing
    .map((p) => (p.data as { displayName?: string })?.displayName ?? "Someone")
    .filter(Boolean);
  const label =
    names.length === 1
      ? `${names[0]} is typing`
      : names.length === 2
        ? `${names[0]} and ${names[1]} are typing`
        : `${names.length} people are typing`;

  return (
    <div className="h-5 shrink-0 px-5 text-xs text-muted-foreground">
      <span className="inline-flex items-center gap-1.5">
        <span className="inline-flex gap-0.5">
          <span className="size-1 animate-bounce rounded-full bg-muted-foreground [animation-delay:-0.3s]" />
          <span className="size-1 animate-bounce rounded-full bg-muted-foreground [animation-delay:-0.15s]" />
          <span className="size-1 animate-bounce rounded-full bg-muted-foreground" />
        </span>
        <span>{label}</span>
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Composer
// ---------------------------------------------------------------------------

function ComposerInner({
  textareaRef,
  value,
  onChange,
  placeholder,
  canSend,
  onSubmit,
  ariaLabel,
}: {
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
  value: string;
  onChange: (v: string) => void;
  placeholder: string;
  canSend: boolean;
  onSubmit: () => void;
  ariaLabel: string;
}) {
  return (
    <div className="flex items-end gap-2 rounded-xl border border-border bg-card px-3 py-2 shadow-sm focus-within:border-ring/80 focus-within:ring-2 focus-within:ring-ring/20">
      <Textarea
        ref={textareaRef}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        rows={1}
        className="min-h-0 resize-none border-0 bg-transparent p-0 shadow-none focus-visible:ring-0"
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            onSubmit();
          }
        }}
      />
      <Button
        type="submit"
        size="icon"
        disabled={!canSend}
        className="size-8 shrink-0 rounded-lg"
        aria-label={ariaLabel}
        title="Send (Enter)"
      >
        <Send className="size-3.5" />
      </Button>
    </div>
  );
}

function Composer({
  channelId,
  currentUser,
}: {
  channelId: string;
  currentUser: User;
}) {
  const { data: channel } = db.useQueryOne<Channel>("Channel", channelId);
  const [body, setBody] = useState("");
  const send = db.useMutation<
    { channelId: string; body: string },
    unknown
  >("sendMessage");
  const { setPresence } = useRoom(`channel:${channelId}`, currentUser.id, {
    initialPresence: { displayName: currentUser.displayName, typing: false },
  });

  const typingTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  function onChange(v: string) {
    setBody(v);
    if (typingTimer.current) clearTimeout(typingTimer.current);
    setPresence({ displayName: currentUser.displayName, typing: true });
    typingTimer.current = setTimeout(() => {
      setPresence({ displayName: currentUser.displayName, typing: false });
    }, 3000);
  }

  async function submit() {
    const text = body.trim();
    if (!text) return;
    setBody("");
    setPresence({ displayName: currentUser.displayName, typing: false });

    const tempId = `tmp_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    const store = db.sync.store;
    store.applyChange({
      seq: 0,
      entity: "Message",
      row_id: tempId,
      kind: "insert",
      data: {
        id: tempId,
        channelId,
        authorId: currentUser.id,
        parentMessageId: null,
        body: text,
        createdAt: new Date().toISOString(),
      },
      timestamp: "",
    });
    store.notify();

    try {
      await send.mutate({ channelId, body: text });
    } catch (e) {
      console.error("send failed", e);
    } finally {
      store.applyChange({
        seq: 0,
        entity: "Message",
        row_id: tempId,
        kind: "delete",
        timestamp: "",
      });
      store.notify();
    }
  }

  const placeholder =
    channel && !isDmChannel(channel)
      ? `Message #${channel.name}`
      : "Message";
  const canSend = body.trim().length > 0;

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 200) + "px";
  }, [body]);

  return (
    <form
      className="shrink-0 px-4 pb-3 pt-1"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <ComposerInner
        textareaRef={textareaRef}
        value={body}
        onChange={onChange}
        placeholder={placeholder}
        canSend={canSend}
        onSubmit={submit}
        ariaLabel="Send message"
      />
    </form>
  );
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

type PaletteItem =
  | { kind: "channel"; id: string; channel: Channel; label: string }
  | { kind: "dm"; id: string; channel: Channel; label: string; peer: User }
  | { kind: "user"; id: string; user: User; label: string };

function CommandPalette({
  currentUser,
  onClose,
  onSelectChannel,
}: {
  currentUser: User;
  onClose: () => void;
  onSelectChannel: (channelId: string) => void;
}) {
  const { data: channels } = db.useQuery<Channel>("Channel");
  const { data: users } = db.useQuery<User>("User");
  const { data: myMemberships } = db.useQuery<Membership>("Membership", {
    where: { userId: currentUser.id },
  });
  const startDm = db.useMutation<
    { otherUserId: string },
    { channelId: string }
  >("startDm");

  const myChannelIds = useMemo(
    () => new Set((myMemberships ?? []).map((m) => m.channelId)),
    [myMemberships],
  );

  const [query, setQuery] = useState("");
  const [sel, setSel] = useState(0);

  const items = useMemo(() => {
    const out: PaletteItem[] = [];
    const dmChannels = new Map<string, Channel>();
    for (const ch of channels ?? []) {
      if (isDmChannel(ch)) {
        if (!myChannelIds.has(ch.id)) continue;
        const peerId = dmPeerId(ch, currentUser.id);
        if (peerId) dmChannels.set(peerId, ch);
      } else {
        out.push({
          kind: "channel",
          id: `channel:${ch.id}`,
          channel: ch,
          label: ch.name,
        });
      }
    }
    for (const u of users ?? []) {
      if (u.id === currentUser.id) continue;
      const ch = dmChannels.get(u.id);
      if (ch) {
        out.push({
          kind: "dm",
          id: `dm:${ch.id}`,
          channel: ch,
          peer: u,
          label: u.displayName,
        });
      } else {
        out.push({
          kind: "user",
          id: `user:${u.id}`,
          user: u,
          label: u.displayName,
        });
      }
    }
    return out;
  }, [channels, users, myChannelIds, currentUser.id]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items.slice(0, 40);
    return items
      .filter((it) => it.label.toLowerCase().includes(q))
      .slice(0, 40);
  }, [items, query]);

  useEffect(() => {
    setSel(0);
  }, [query, filtered.length]);

  async function activate(item: PaletteItem) {
    if (item.kind === "channel" || item.kind === "dm") {
      onSelectChannel(item.channel.id);
      return;
    }
    try {
      const res = await startDm.mutate({ otherUserId: item.user.id });
      if (res?.channelId) onSelectChannel(res.channelId);
    } catch (e) {
      console.error(e);
    }
  }

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSel((s) => Math.min(s + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSel((s) => Math.max(s - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = filtered[sel];
      if (item) void activate(item);
    }
  };

  const channelsOut = filtered.filter((i) => i.kind === "channel");
  const dmsOut = filtered.filter((i) => i.kind === "dm");
  const usersOut = filtered.filter((i) => i.kind === "user");
  const flatIndex = (item: PaletteItem) =>
    filtered.findIndex((x) => x.id === item.id);

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-start bg-black/50 pt-[14vh] backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="mx-auto w-full max-w-lg overflow-hidden rounded-xl border border-border bg-popover text-popover-foreground shadow-xl"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="flex items-center gap-2 border-b border-border px-3 py-2.5">
          <Search className="size-4 text-muted-foreground" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            placeholder="Jump to channel, DM, or person…"
            className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
          <Kbd>Esc</Kbd>
        </div>
        <div className="max-h-[50vh] overflow-y-auto py-1.5">
          {filtered.length === 0 ? (
            <div className="px-4 py-6 text-center text-sm text-muted-foreground">
              No matches.
            </div>
          ) : (
            <>
              {channelsOut.length > 0 && (
                <>
                  <PaletteSection label="Channels" />
                  {channelsOut.map((it) => (
                    <PaletteRow
                      key={it.id}
                      item={it}
                      selected={flatIndex(it) === sel}
                      onActivate={() => void activate(it)}
                      onHover={() => setSel(flatIndex(it))}
                    />
                  ))}
                </>
              )}
              {dmsOut.length > 0 && (
                <>
                  <PaletteSection label="Direct messages" />
                  {dmsOut.map((it) => (
                    <PaletteRow
                      key={it.id}
                      item={it}
                      selected={flatIndex(it) === sel}
                      onActivate={() => void activate(it)}
                      onHover={() => setSel(flatIndex(it))}
                    />
                  ))}
                </>
              )}
              {usersOut.length > 0 && (
                <>
                  <PaletteSection label="People" />
                  {usersOut.map((it) => (
                    <PaletteRow
                      key={it.id}
                      item={it}
                      selected={flatIndex(it) === sel}
                      onActivate={() => void activate(it)}
                      onHover={() => setSel(flatIndex(it))}
                    />
                  ))}
                </>
              )}
            </>
          )}
        </div>
        <div className="flex items-center gap-3 border-t border-border bg-muted/30 px-3 py-2 text-[11px] text-muted-foreground">
          <span className="inline-flex items-center gap-1">
            <Kbd>↑</Kbd>
            <Kbd>↓</Kbd> navigate
          </span>
          <span className="inline-flex items-center gap-1">
            <Kbd>↵</Kbd> select
          </span>
          <span className="ml-auto inline-flex items-center gap-1">
            <Kbd>⌘/</Kbd> shortcuts
          </span>
        </div>
      </div>
    </div>
  );
}

function PaletteSection({ label }: { label: string }) {
  return (
    <div className="px-3 pt-2 pb-0.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
      {label}
    </div>
  );
}

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="inline-flex min-w-[18px] items-center justify-center rounded border border-border bg-card px-1.5 py-0.5 font-mono text-[10px] font-semibold text-muted-foreground shadow-[0_1px_0_oklch(from_var(--color-border)_l_c_h/0.8)]">
      {children}
    </kbd>
  );
}

function PaletteRow({
  item,
  selected,
  onActivate,
  onHover,
}: {
  item: PaletteItem;
  selected: boolean;
  onActivate: () => void;
  onHover: () => void;
}) {
  return (
    <div
      className={cn(
        "flex cursor-pointer items-center gap-2.5 px-3 py-2",
        selected && "bg-accent",
      )}
      onClick={onActivate}
      onMouseEnter={onHover}
      role="option"
      aria-selected={selected}
    >
      <div className="flex size-6 items-center justify-center text-muted-foreground">
        {item.kind === "channel" ? (
          item.channel.isPrivate ? (
            <Lock className="size-3.5" />
          ) : (
            <Hash className="size-4" />
          )
        ) : item.kind === "dm" ? (
          <ColorAvatar name={item.peer.displayName} color={item.peer.avatarColor} size="sm" />
        ) : (
          <ColorAvatar name={item.user.displayName} color={item.user.avatarColor} size="sm" />
        )}
      </div>
      <div className="flex-1 truncate text-sm">{item.label}</div>
      <div className="text-[11px] text-muted-foreground">
        {item.kind === "channel"
          ? "Channel"
          : item.kind === "dm"
            ? "Direct message"
            : "Start DM"}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Shortcut help
// ---------------------------------------------------------------------------

function ShortcutsHelp({ onClose }: { onClose: () => void }) {
  const mod = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform)
    ? "⌘"
    : "Ctrl";
  const rows: { label: string; keys: string[] }[] = [
    { label: "Quick switcher", keys: [mod, "K"] },
    { label: "Start a direct message", keys: [mod, "Shift", "D"] },
    { label: "Close panel / dismiss", keys: ["Esc"] },
    { label: "Send message", keys: ["Enter"] },
    { label: "Newline in composer", keys: ["Shift", "Enter"] },
    { label: "Shortcut help", keys: [mod, "/"] },
  ];
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Keyboard shortcuts</DialogTitle>
          <DialogDescription>
            Fly around without touching a mouse.
          </DialogDescription>
        </DialogHeader>
        <div className="grid grid-cols-[1fr_auto] items-center gap-x-6 gap-y-2 py-1">
          {rows.map((r) => (
            <React.Fragment key={r.label}>
              <div className="text-sm text-muted-foreground">{r.label}</div>
              <div className="flex items-center gap-1">
                {r.keys.map((k) => (
                  <Kbd key={k}>{k}</Kbd>
                ))}
              </div>
            </React.Fragment>
          ))}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Profile modal
// ---------------------------------------------------------------------------

const AVATAR_COLORS = [
  "#8b5cf6", "#6366f1", "#3b82f6", "#06b6d4", "#10b981",
  "#84cc16", "#eab308", "#f97316", "#ef4444", "#ec4899",
  "#d946ef", "#71717a",
];

function ProfileModal({
  userId,
  currentUser,
  onClose,
  onStartDm,
}: {
  userId: string;
  currentUser: User;
  onClose: () => void;
  onStartDm: (channelId: string) => void;
}) {
  const { data: user } = db.useQueryOne<User>("User", userId);
  const isMe = userId === currentUser.id;

  const update = db.useMutation<
    { displayName?: string; email?: string; avatarColor?: string },
    { userId: string; changed: boolean }
  >("updateProfile");
  const startDm = db.useMutation<
    { otherUserId: string },
    { channelId: string }
  >("startDm");

  const [editing, setEditing] = useState(false);
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [color, setColor] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (user) {
      setName(user.displayName);
      setEmail(user.email);
      setColor(user.avatarColor || "#8b5cf6");
    }
  }, [user?.displayName, user?.email, user?.avatarColor]);

  async function save() {
    if (!user) return;
    setBusy(true);
    setErr(null);
    try {
      await update.mutate({
        displayName: name,
        email,
        avatarColor: color,
      });
      if (isMe) {
        const next = {
          ...currentUser,
          displayName: name,
          email,
          avatarColor: color,
        };
        localStorage.setItem(storageKey("user"), JSON.stringify(next));
      }
      setEditing(false);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function dm() {
    try {
      const res = await startDm.mutate({ otherUserId: userId });
      if (res?.channelId) onStartDm(res.channelId);
    } catch (e) {
      console.error(e);
    }
  }

  if (!user) {
    return (
      <Dialog open onOpenChange={(o) => !o && onClose()}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Profile</DialogTitle>
            <DialogDescription>Loading…</DialogDescription>
          </DialogHeader>
        </DialogContent>
      </Dialog>
    );
  }

  const displayColor = editing ? color : user.avatarColor || "#8b5cf6";
  const displayName = editing ? name : user.displayName;

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-3">
            <ColorAvatar name={displayName} color={displayColor} size="lg" />
            <div className="min-w-0 flex-1">
              <DialogTitle className="flex items-baseline gap-2">
                <span className="truncate">{displayName}</span>
                {isMe && (
                  <span className="text-sm font-normal text-muted-foreground">
                    (you)
                  </span>
                )}
              </DialogTitle>
              <DialogDescription>{user.email}</DialogDescription>
            </div>
          </div>
        </DialogHeader>

        {editing ? (
          <div className="grid gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="profile-name">Display name</Label>
              <Input
                id="profile-name"
                autoFocus
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="profile-email">Email</Label>
              <Input
                id="profile-email"
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            </div>
            <div className="grid gap-1.5">
              <Label>Avatar color</Label>
              <div className="flex flex-wrap gap-1.5">
                {AVATAR_COLORS.map((c) => (
                  <button
                    key={c}
                    type="button"
                    onClick={() => setColor(c)}
                    aria-label={`Color ${c}`}
                    className={cn(
                      "size-7 rounded-full ring-offset-background transition-all hover:scale-110",
                      color.toLowerCase() === c.toLowerCase() &&
                        "ring-2 ring-ring ring-offset-2",
                    )}
                    style={{ backgroundColor: c }}
                  />
                ))}
              </div>
            </div>
            {err && (
              <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
                {err}
              </div>
            )}
          </div>
        ) : (
          <div className="grid gap-3">
            <div>
              <Label className="text-xs uppercase tracking-wider text-muted-foreground">
                Email
              </Label>
              <div className="mt-1 text-sm">{user.email}</div>
            </div>
          </div>
        )}

        <DialogFooter>
          {!isMe && !editing && (
            <Button variant="outline" onClick={() => void dm()}>
              Send message
            </Button>
          )}
          {isMe && !editing && (
            <Button variant="outline" onClick={() => setEditing(true)}>
              Edit profile
            </Button>
          )}
          {editing && (
            <>
              <Button
                variant="outline"
                onClick={() => {
                  setEditing(false);
                  setErr(null);
                }}
                disabled={busy}
              >
                Cancel
              </Button>
              <Button onClick={() => void save()} disabled={busy}>
                {busy && <Loader2 className="size-4 animate-spin" />}
                {busy ? "Saving…" : "Save"}
              </Button>
            </>
          )}
          {!editing && (
            <Button variant="outline" onClick={onClose}>
              Close
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Channel details modal
// ---------------------------------------------------------------------------

function ChannelDetailsModal({
  channelId,
  currentUser,
  onClose,
  onOpenProfile,
}: {
  channelId: string;
  currentUser: User;
  onClose: () => void;
  onOpenProfile: (userId: string) => void;
}) {
  const { data: channel } = db.useQueryOne<Channel>("Channel", channelId);
  const { data: creator } = db.useQueryOne<User>(
    "User",
    channel?.createdBy ?? "",
  );
  const { data: memberships } = db.useQuery<Membership>("Membership", {
    where: { channelId },
  });

  const update = db.useMutation<
    {
      channelId: string;
      name?: string;
      topic?: string;
      isPrivate?: boolean;
    },
    { channelId: string; changed: boolean }
  >("updateChannel");

  const [editing, setEditing] = useState(false);
  const [name, setName] = useState("");
  const [topic, setTopic] = useState("");
  const [isPrivate, setIsPrivate] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (channel) {
      setName(channel.name);
      setTopic(channel.topic ?? "");
      setIsPrivate(Boolean(channel.isPrivate));
    }
  }, [channel?.name, channel?.topic, channel?.isPrivate]);

  if (!channel) {
    return (
      <Dialog open onOpenChange={(o) => !o && onClose()}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Channel</DialogTitle>
            <DialogDescription>Loading…</DialogDescription>
          </DialogHeader>
        </DialogContent>
      </Dialog>
    );
  }

  const canEdit = channel.createdBy === currentUser.id;

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      await update.mutate({
        channelId,
        name,
        topic,
        isPrivate,
      });
      setEditing(false);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (/UNIQUE constraint/i.test(msg)) {
        setErr(`#${name} already exists`);
      } else if (/INVALID_NAME|lowercase/i.test(msg)) {
        setErr("Use lowercase letters, numbers, and dashes only.");
      } else {
        setErr(msg);
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div
              className="grid size-14 shrink-0 place-items-center rounded-full text-xl font-semibold"
              style={{
                background: channel.isPrivate
                  ? "linear-gradient(135deg, #6366f1, #8b5cf6)"
                  : "var(--color-accent)",
                color: channel.isPrivate ? "white" : "var(--color-foreground)",
              }}
            >
              {channel.isPrivate ? "🔒" : "#"}
            </div>
            <div className="min-w-0 flex-1">
              <DialogTitle className="truncate">
                {channel.isPrivate ? "🔒 " : "# "}
                {channel.name}
              </DialogTitle>
              <DialogDescription>
                {channel.isPrivate ? "Private channel" : "Public channel"}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        {editing ? (
          <div className="grid gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="channel-edit-name">Name</Label>
              <Input
                id="channel-edit-name"
                autoFocus
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="channel-edit-topic">Topic</Label>
              <Input
                id="channel-edit-topic"
                value={topic}
                onChange={(e) => setTopic(e.target.value)}
                placeholder="What's this channel about?"
              />
            </div>
            <label className="flex cursor-pointer items-start gap-3 rounded-lg border border-border bg-accent/30 px-3 py-2.5">
              <Checkbox
                checked={isPrivate}
                onCheckedChange={(v) => setIsPrivate(!!v)}
                className="mt-0.5"
              />
              <div>
                <div className="text-sm font-medium">Private channel</div>
                <div className="mt-0.5 text-xs text-muted-foreground">
                  Only invited members can see or join.
                </div>
              </div>
            </label>
            {err && (
              <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
                {err}
              </div>
            )}
          </div>
        ) : (
          <div className="grid gap-4">
            <div>
              <Label className="text-xs uppercase tracking-wider text-muted-foreground">
                Topic
              </Label>
              <div
                className={cn(
                  "mt-1 text-sm",
                  !channel.topic && "italic text-muted-foreground",
                )}
              >
                {channel.topic || "No topic set."}
              </div>
            </div>
            <div>
              <Label className="text-xs uppercase tracking-wider text-muted-foreground">
                Created by
              </Label>
              <button
                className="mt-1 flex items-center gap-2 rounded-md px-1 py-0.5 hover:bg-accent"
                onClick={() => creator && onOpenProfile(creator.id)}
              >
                <ColorAvatar
                  name={creator?.displayName}
                  color={creator?.avatarColor}
                  size="sm"
                />
                <span className="text-sm">
                  {creator?.displayName ?? "Unknown"}
                </span>
              </button>
            </div>
            <div>
              <Label className="text-xs uppercase tracking-wider text-muted-foreground">
                Members · {(memberships ?? []).length}
              </Label>
              <div className="mt-1 max-h-[240px] overflow-y-auto">
                {(memberships ?? []).length === 0 ? (
                  <div className="p-2 text-sm text-muted-foreground">
                    No explicit members — anyone in the workspace can join.
                  </div>
                ) : (
                  (memberships ?? []).map((m) => (
                    <MemberRow
                      key={m.id}
                      userId={m.userId}
                      role={m.role}
                      onClick={() => onOpenProfile(m.userId)}
                    />
                  ))
                )}
              </div>
            </div>
          </div>
        )}

        <DialogFooter>
          {canEdit && !editing && (
            <Button variant="outline" onClick={() => setEditing(true)}>
              Edit channel
            </Button>
          )}
          {editing && (
            <>
              <Button
                variant="outline"
                onClick={() => {
                  setEditing(false);
                  setErr(null);
                }}
                disabled={busy}
              >
                Cancel
              </Button>
              <Button onClick={() => void save()} disabled={busy}>
                {busy && <Loader2 className="size-4 animate-spin" />}
                {busy ? "Saving…" : "Save"}
              </Button>
            </>
          )}
          {!editing && (
            <Button variant="outline" onClick={onClose}>
              Close
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function MemberRow({
  userId,
  role,
  onClick,
}: {
  userId: string;
  role: string;
  onClick: () => void;
}) {
  const { data: user } = db.useQueryOne<User>("User", userId);
  return (
    <button
      onClick={onClick}
      className="flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left hover:bg-accent"
    >
      <ColorAvatar name={user?.displayName} color={user?.avatarColor} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium">
          {user?.displayName ?? "…"}
        </div>
        <div className="truncate text-xs text-muted-foreground">
          {user?.email ?? ""}
        </div>
      </div>
      {role && (
        <Badge variant="secondary" className="text-[10px] uppercase tracking-wider">
          {role}
        </Badge>
      )}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

function EmptyState({ title, body }: { title: string; body: string }) {
  return (
    <div className="grid flex-1 place-items-center p-10 text-center">
      <div>
        <div className="mb-1 text-lg font-semibold">{title}</div>
        <div className="max-w-sm text-sm text-muted-foreground">{body}</div>
      </div>
    </div>
  );
}
