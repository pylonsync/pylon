/**
 * Statecraft chat demo.
 *
 * Exercises: auth sessions, live sync (messages), rooms (typing +
 * presence), reactions via toggleReaction, optimistic sends via
 * useMutation, threads, DMs. Two browser windows side-by-side give you
 * the full multiplayer experience.
 */

import React, { useEffect, useMemo, useRef, useState } from "react";
import { init, db, useRoom, callFn, configureClient } from "@statecraft/react";

const BASE_URL = "http://localhost:4321";
init({ baseUrl: BASE_URL });
configureClient({ baseUrl: BASE_URL });

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
  // Code blocks first — pull them out so their content isn't touched by
  // the inline passes.
  const blocks: string[] = [];
  let work = body.replace(/```([\s\S]*?)```/g, (_, code: string) => {
    const idx = blocks.push(code) - 1;
    return `\u0000BLOCK${idx}\u0000`;
  });
  work = escapeHtml(work);
  // Inline code
  work = work.replace(/`([^`]+)`/g, (_, c) => `<code>${c}</code>`);
  // Bold / italic (greedy but scoped to same line).
  work = work.replace(/\*([^*\n]+)\*/g, "<strong>$1</strong>");
  work = work.replace(/(^|[^\w])_([^_\n]+)_(?=[^\w]|$)/g, "$1<em>$2</em>");
  // URLs — match http(s), skip if it's inside quoted attribute.
  work = work.replace(
    /(^|[\s(])((?:https?:\/\/)[^\s<>"']+)/g,
    (_, lead, url) =>
      `${lead}<a href="${url}" target="_blank" rel="noopener noreferrer">${url}</a>`,
  );
  // Restore code blocks (escaped).
  work = work.replace(/\u0000BLOCK(\d+)\u0000/g, (_, idx) => {
    const raw = blocks[Number(idx)];
    return `<pre><code>${escapeHtml(raw.replace(/^\n/, ""))}</code></pre>`;
  });
  return work;
}

// Shared renderer used by main + thread message rows.
function RichBody({ body }: { body: string }) {
  return (
    <div
      className="message-body"
      dangerouslySetInnerHTML={{ __html: renderMarkdown(body) }}
    />
  );
}

function dmPeerId(ch: { name: string }, me: string): string | null {
  if (!isDmChannel(ch)) return null;
  const [, a, b] = ch.name.split(":");
  return a === me ? b : a === undefined ? null : a;
}

// Star persistence — per-user localStorage so stars don't bleed across
// accounts on a shared machine. Not synced across devices; move to a
// StarredChannel entity if you need that.
function starsKey(userId: string) {
  return `statecraft_stars_${userId}`;
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
// Root
// ---------------------------------------------------------------------------

export function ChatApp() {
  const [currentUser, setCurrentUser] = useState<User | null>(() => {
    try {
      const token = localStorage.getItem("statecraft_token");
      const cached = localStorage.getItem("statecraft_user");
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
      localStorage.setItem("statecraft_user", JSON.stringify(liveUser));
    }
  }, [liveUser, currentUser?.id]);

  useEffect(() => {
    const token = localStorage.getItem("statecraft_token");
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
          localStorage.removeItem("statecraft_token");
          localStorage.removeItem("statecraft_user");
          try {
            indexedDB.deleteDatabase("statecraft_sync_default");
          } catch {}
          setCurrentUser(null);
        }
      } catch {}
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Close the thread panel when switching channels — a thread parent
  // belongs to one channel; leaving that channel makes the panel stale.
  useEffect(() => {
    setThreadMessageId(null);
  }, [activeChannelId]);

  // Global keyboard shortcuts. Centralized here so every shortcut has one
  // obvious home — easier to reason about conflicts, and one cleanup path.
  useEffect(() => {
    if (!currentUser) return;
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      // ⌘K — command palette
      if (mod && !e.shiftKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen(true);
        return;
      }
      // ⌘Shift+D — new DM picker
      if (mod && e.shiftKey && e.key.toLowerCase() === "d") {
        e.preventDefault();
        setDmPickerOpen(true);
        return;
      }
      // ⌘/ — shortcut help
      if (mod && e.key === "/") {
        e.preventDefault();
        setShortcutsOpen((v) => !v);
        return;
      }
      // Esc — peel overlays in a predictable order: palette > DM picker >
      // shortcut help > thread panel.
      if (e.key === "Escape") {
        if (paletteOpen) {
          setPaletteOpen(false);
          return;
        }
        if (dmPickerOpen) {
          setDmPickerOpen(false);
          return;
        }
        if (shortcutsOpen) {
          setShortcutsOpen(false);
          return;
        }
        if (threadMessageId) {
          setThreadMessageId(null);
          return;
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [currentUser, paletteOpen, dmPickerOpen, shortcutsOpen, threadMessageId]);

  async function signOut() {
    const token = localStorage.getItem("statecraft_token");
    localStorage.removeItem("statecraft_token");
    localStorage.removeItem("statecraft_user");
    if (token) {
      fetch(`${BASE_URL}/api/auth/session`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      }).catch(() => {});
    }
    try {
      indexedDB.deleteDatabase("statecraft_sync_default");
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
    <div className="app">
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
        <div className="main">
          <EmptyState
            title="Welcome to Statecraft Chat"
            body="Pick a channel on the left or start a direct message."
          />
        </div>
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

// UIContext — short-path for deep children (message rows, sidebar user chip)
// to pop open profile / channel-details modals without threading the
// openers through every prop. Kept local to this file; real apps would
// probably split this into smaller contexts.

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
  const [email, setEmail] = useState("alice@example.com");
  const [name, setName] = useState("Alice");
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function go() {
    setLoading(true);
    setErr(null);
    try {
      const session = await fetch(`${BASE_URL}/api/auth/guest`, {
        method: "POST",
      }).then((r) => r.json());
      const token: string = session.token;
      localStorage.setItem("statecraft_token", token);
      configureClient({ baseUrl: BASE_URL });
      const user = await callFn<User>("upsertUser", {
        email,
        displayName: name,
      });
      await fetch(`${BASE_URL}/api/auth/upgrade`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({ user_id: user.id }),
      });
      localStorage.setItem("statecraft_user", JSON.stringify(user));
      void db.sync.pull();
      onReady(user);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="login">
      <div className="login-card">
        <div className="login-logo" aria-hidden="true">
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none">
            <path
              d="M4 7l8-4 8 4v10l-8 4-8-4V7z"
              stroke="white"
              strokeWidth="2"
              strokeLinejoin="round"
            />
            <path
              d="M12 11v10M4 7l8 4 8-4"
              stroke="white"
              strokeWidth="2"
              strokeLinejoin="round"
            />
          </svg>
        </div>
        <div className="login-title">Sign in to Statecraft</div>
        <div className="login-subtitle">
          Local-first chat, powered by live sync.
        </div>
        <label className="field">
          <span className="field-label">Email</span>
          <input
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="you@example.com"
            className="input"
            autoFocus
          />
        </label>
        <label className="field">
          <span className="field-label">Display name</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Alice"
            className="input"
            onKeyDown={(e) => {
              if (e.key === "Enter") void go();
            }}
          />
        </label>
        {err && <div className="login-error">{err}</div>}
        <button onClick={go} disabled={loading} className="btn-primary">
          {loading ? "Signing in…" : "Continue"}
        </button>
        <div className="login-footer">
          Demo-only. Real deploys wire up magic codes or OAuth.
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Unread-count hook — single source of truth for sidebar badges.
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

  // Split into three buckets: starred (regular channels only), non-starred
  // public channels, and DMs (private channels with the dm: name prefix
  // that the current user is a member of).
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
      <aside className="sidebar">
        <div
          className="sidebar-user clickable"
          onClick={() => ui.openProfile(currentUser.id)}
          role="button"
          tabIndex={0}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              ui.openProfile(currentUser.id);
            }
          }}
        >
          <div
            className="avatar avatar-md avatar-online-ring"
            style={{ backgroundColor: currentUser.avatarColor || "#8b5cf6" }}
            aria-hidden="true"
          >
            {initials(currentUser.displayName)}
          </div>
          <div className="sidebar-user-meta">
            <div className="sidebar-user-name">{currentUser.displayName}</div>
            <div className="sidebar-user-status">Online</div>
          </div>
          <button
            onClick={(e) => {
              e.stopPropagation();
              onSignOut();
            }}
            className="icon-btn"
            title="Sign out"
            aria-label="Sign out"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
              <path
                d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4M10 17l5-5-5-5M15 12H3"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        </div>

        <nav className="sidebar-list">
          {starred.length > 0 && (
            <>
              <div className="sidebar-section">
                <span>Starred</span>
              </div>
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
            </>
          )}

          <div className="sidebar-section">
            <span>
              Channels{" "}
              <span style={{ color: "var(--text-dim)", fontWeight: 400 }}>
                {regular.length}
              </span>
            </span>
            <button
              onClick={() => setCreateModalOpen(true)}
              className="icon-btn"
              title="Create channel"
              aria-label="Create channel"
              style={{ width: 20, height: 20 }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none">
                <path
                  d="M12 5v14M5 12h14"
                  stroke="currentColor"
                  strokeWidth="2.2"
                  strokeLinecap="round"
                />
              </svg>
            </button>
          </div>
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

          <div className="sidebar-section">
            <span>Direct Messages</span>
            <button
              onClick={() => setDmPickerOpen(true)}
              className="icon-btn"
              title="Start a DM"
              aria-label="Start a DM"
              style={{ width: 20, height: 20 }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none">
                <path
                  d="M12 5v14M5 12h14"
                  stroke="currentColor"
                  strokeWidth="2.2"
                  strokeLinecap="round"
                />
              </svg>
            </button>
          </div>
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
            <div
              style={{
                padding: "2px 10px 8px",
                fontSize: 12,
                color: "var(--text-dim)",
              }}
            >
              No DMs yet.
            </div>
          )}
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
      className={
        "channel-btn" +
        (active ? " active" : "") +
        (unread > 0 ? " unread" : "")
      }
      onClick={onSelect}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <span className="channel-prefix">
        {channel.isPrivate ? "🔒" : "#"}
      </span>
      <span className="channel-name">{channel.name}</span>
      {unread > 0 && <span className="unread-badge">{unread}</span>}
      <button
        type="button"
        className={"channel-star" + (starred ? " starred" : "")}
        onClick={(e) => {
          e.stopPropagation();
          onToggleStar();
        }}
        title={starred ? "Unstar" : "Star"}
        aria-label={starred ? "Unstar" : "Star"}
      >
        <svg viewBox="0 0 24 24" fill={starred ? "currentColor" : "none"}>
          <path
            d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinejoin="round"
          />
        </svg>
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
      className={
        "channel-btn" +
        (active ? " active" : "") +
        (unread > 0 ? " unread" : "")
      }
      onClick={onSelect}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <span className="dm-presence-dot online" />
      <span className="channel-name">{label}</span>
      {unread > 0 && <span className="unread-badge">{unread}</span>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Create-channel modal — name + public/private toggle
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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

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
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ width: 420 }}
        role="dialog"
        aria-modal="true"
      >
        <div className="modal-title">Create a channel</div>
        <div className="modal-subtitle">
          Channels are where conversations happen around a topic.
        </div>
        <label className="field">
          <span className="field-label">Name</span>
          <input
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
            className="input"
          />
        </label>
        <div
          style={{
            display: "flex",
            alignItems: "flex-start",
            gap: 10,
            padding: "10px 12px",
            marginTop: 4,
            background: "var(--surface-raised)",
            border: "1px solid var(--border)",
            borderRadius: 10,
            cursor: "pointer",
            userSelect: "none",
          }}
          onClick={() => setIsPrivate((v) => !v)}
        >
          <input
            type="checkbox"
            checked={isPrivate}
            onChange={(e) => setIsPrivate(e.target.checked)}
            onClick={(e) => e.stopPropagation()}
            style={{
              accentColor: "var(--accent)",
              marginTop: 2,
              flexShrink: 0,
            }}
          />
          <div>
            <div style={{ fontSize: 13.5, fontWeight: 500 }}>
              {isPrivate ? "🔒 Private channel" : "# Public channel"}
            </div>
            <div
              style={{
                fontSize: 12,
                color: "var(--text-muted)",
                marginTop: 2,
              }}
            >
              {isPrivate
                ? "Only invited members can see or join."
                : "Anyone in the workspace can see and join."}
            </div>
          </div>
        </div>
        {err && <div className="login-error">{err}</div>}
        <div className="modal-footer">
          <button className="btn-secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn-primary"
            style={{ margin: 0, width: "auto", padding: "8px 18px" }}
            onClick={() => void submit()}
            disabled={busy || name.trim().length === 0}
          >
            {busy ? "Creating…" : "Create"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// DM picker modal — pick a user to open a DM with.
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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="modal-title">Start a direct message</div>
        <div className="modal-subtitle">Pick someone to chat with.</div>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search by name or email…"
          className="input"
          autoFocus
          style={{ marginBottom: 10 }}
        />
        <div className="user-list">
          {filtered.length === 0 ? (
            <div
              style={{
                padding: "16px 10px",
                fontSize: 13,
                color: "var(--text-dim)",
              }}
            >
              No users match.
            </div>
          ) : (
            filtered.map((u) => (
              <button
                key={u.id}
                className="user-row"
                onClick={() => void open(u)}
                disabled={opening === u.id}
              >
                <div
                  className="avatar avatar-md"
                  style={{ backgroundColor: u.avatarColor || "#8b5cf6" }}
                >
                  {initials(u.displayName)}
                </div>
                <div className="user-row-meta">
                  <div className="user-row-name">{u.displayName}</div>
                  <div className="user-row-email">{u.email}</div>
                </div>
              </button>
            ))
          )}
        </div>
        <div className="modal-footer">
          <button className="btn-secondary" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>
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

  if (!channel) return <div className="main" />;

  const isDm = isDmChannel(channel);
  const ui = React.useContext(UIContext);

  return (
    <main className="main">
      <header className="channel-header">
        <div className="channel-title-group">
          {isDm ? (
            <DmHeader channel={channel} currentUser={currentUser} />
          ) : (
            <>
              <span
                className="channel-title channel-header-title"
                onClick={() => ui.openChannelDetails(channel.id)}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    ui.openChannelDetails(channel.id);
                  }
                }}
                title="Channel details"
              >
                <span className="channel-title-prefix">
                  {channel.isPrivate ? "🔒 " : "# "}
                </span>
                {channel.name}
              </span>
              {channel.topic && (
                <>
                  <span className="channel-divider">·</span>
                  <span className="channel-topic">{channel.topic}</span>
                </>
              )}
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
    <span
      className="channel-title channel-header-title"
      onClick={() => peerId && ui.openProfile(peerId)}
      role="button"
      tabIndex={0}
      title="View profile"
    >
      <span style={{ marginRight: 8 }}>
        <span
          className="avatar avatar-sm"
          style={{
            display: "inline-flex",
            verticalAlign: "middle",
            backgroundColor: peer?.avatarColor || "#8b5cf6",
          }}
        >
          {initials(peer?.displayName)}
        </span>
      </span>
      {peer?.displayName ?? "Direct message"}
    </span>
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

  // Click-outside to close the popover.
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
    <div className="popover-wrap" ref={wrapRef}>
      <button
        type="button"
        className="presence-btn"
        onClick={() => setOpen((v) => !v)}
        title="Who's here"
      >
        <span className="presence-dot" />
        <span>{others.length === 0 ? "Just you" : `${total} here`}</span>
      </button>
      {open && (
        <div className="popover">
          <div className="popover-header">In this channel</div>
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
    <div className="popover-item">
      <div
        className="avatar avatar-sm"
        style={{ backgroundColor: display?.avatarColor || "#8b5cf6" }}
      >
        {initials(display?.displayName)}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            fontSize: 13,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {display?.displayName ?? "…"}
          {isMe && (
            <span style={{ color: "var(--text-dim)", marginLeft: 6 }}>
              (you)
            </span>
          )}
        </div>
      </div>
      <span className="popover-item-status">●</span>
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
  // Only top-level messages (no parent) in the main list. Thread replies
  // surface in the thread panel.
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
  // Reply counts per parent id — lets a message know how many replies it has
  // without a per-row query. Counted client-side from the same sync store.
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

  // Track whether the user is pinned to the bottom. If yes, auto-scroll on
  // new messages. If not, we show the "Jump to latest" button instead — no
  // yanking the scroll out from under someone reading older history.
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

  // Scroll on channel switch — force to bottom regardless of prior scroll.
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
      <div className="messages" ref={scrollRef}>
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
        <div key={`day-${m.id}`} className="date-divider">
          <div className="date-divider-line" />
          <div className="date-divider-label">
            {formatDateHeading(m.createdAt)}
          </div>
          <div className="date-divider-line" />
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
    <>
      <div ref={scrollRef} className="messages">
        {rows}
      </div>
      <button
        className={"jump-bottom" + (atBottom ? "" : " visible")}
        onClick={() => {
          scrollRef.current?.scrollTo({
            top: scrollRef.current.scrollHeight,
            behavior: "smooth",
          });
        }}
        aria-label="Jump to latest"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none">
          <path
            d="M12 5v14M5 12l7 7 7-7"
            stroke="currentColor"
            strokeWidth="2.2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
        Jump to latest
      </button>
    </>
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

  const rowClass =
    "message-row" + (compact ? " compact" : " first-in-group");
  const ui = React.useContext(UIContext);
  const openAuthor = () => ui.openProfile(message.authorId);

  return (
    <div className={rowClass}>
      {compact ? (
        <div className="message-timestamp-hover">
          {formatTime(message.createdAt)}
        </div>
      ) : (
        <div className="message-avatar-inline">
          <div
            className="avatar avatar-md clickable"
            style={{ backgroundColor: author?.avatarColor || "#8b5cf6" }}
            onClick={openAuthor}
            role="button"
            tabIndex={0}
            title="View profile"
          >
            {initials(author?.displayName)}
          </div>
        </div>
      )}
      <div className="message-content">
        {!compact && (
          <div className="message-meta">
            <span
              className="message-author clickable"
              onClick={openAuthor}
              role="button"
              tabIndex={0}
              title="View profile"
            >
              {author?.displayName ?? "…"}
            </span>
            <span className="message-time">
              {formatTime(message.createdAt)}
            </span>
          </div>
        )}
        <RichBody body={message.body} />
        {message.editedAt && (
          <span className="message-edited">(edited)</span>
        )}
        {grouped.length > 0 && (
          <div className="reactions">
            {grouped.map(([emoji, { count, mine }]) => (
              <button
                key={emoji}
                onClick={() =>
                  void toggle.mutate({ messageId: message.id, emoji })
                }
                className={"reaction" + (mine ? " mine" : "")}
              >
                <span>{emoji}</span>
                <span className="reaction-count">{count}</span>
              </button>
            ))}
          </div>
        )}
        {replyCount > 0 && (
          <button
            className="reply-count"
            onClick={onOpenThread}
            title="Open thread"
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none">
              <path
                d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
            {replyCount} {replyCount === 1 ? "reply" : "replies"}
          </button>
        )}
      </div>
      <div className="message-actions" aria-hidden="true">
        <button
          onClick={() =>
            void toggle.mutate({ messageId: message.id, emoji: "👍" })
          }
          className="action-btn"
          title="React 👍"
        >
          👍
        </button>
        <button
          onClick={() =>
            void toggle.mutate({ messageId: message.id, emoji: "❤️" })
          }
          className="action-btn"
          title="React ❤️"
        >
          ❤️
        </button>
        <button
          onClick={() =>
            void toggle.mutate({ messageId: message.id, emoji: "🎉" })
          }
          className="action-btn"
          title="React 🎉"
        >
          🎉
        </button>
        <button
          onClick={onOpenThread}
          className="action-btn"
          title={threadOpen ? "Close thread" : "Reply in thread"}
          style={threadOpen ? { color: "var(--accent)" } : undefined}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
            <path
              d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
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
      <aside className="thread-panel">
        <header className="thread-header">
          <div>
            <div className="thread-title">Thread</div>
          </div>
          <button
            onClick={onClose}
            className="icon-btn"
            title="Close"
            aria-label="Close thread"
          >
            <CloseIcon />
          </button>
        </header>
        <div className="empty" style={{ padding: 40 }}>
          <div className="empty-body">Loading…</div>
        </div>
      </aside>
    );
  }

  const replyList = replies ?? [];

  return (
    <aside className="thread-panel">
      <header className="thread-header">
        <div>
          <div className="thread-title">Thread</div>
          <div className="thread-subtitle">
            {replyList.length} {replyList.length === 1 ? "reply" : "replies"}
          </div>
        </div>
        <button
          onClick={onClose}
          className="icon-btn"
          title="Close"
          aria-label="Close thread"
        >
          <CloseIcon />
        </button>
      </header>
      <div className="thread-body" ref={scrollRef}>
        <div className="thread-parent">
          <div className="message-row first-in-group" style={{ padding: 0 }}>
            <div className="message-avatar-inline">
              <div
                className="avatar avatar-md"
                style={{
                  backgroundColor: parentAuthor?.avatarColor || "#8b5cf6",
                }}
              >
                {initials(parentAuthor?.displayName)}
              </div>
            </div>
            <div className="message-content">
              <div className="message-meta">
                <span className="message-author">
                  {parentAuthor?.displayName ?? "…"}
                </span>
                <span className="message-time">
                  {formatTime(parent.createdAt)}
                </span>
              </div>
              <RichBody body={parent.body} />
            </div>
          </div>
        </div>
        <div className="thread-replies">
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
      <div className="thread-composer">
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

  const rowClass =
    "message-row" + (compact ? " compact" : " first-in-group");

  return (
    <div className={rowClass}>
      {compact ? (
        <div className="message-timestamp-hover">
          {formatTime(message.createdAt)}
        </div>
      ) : (
        <div className="message-avatar-inline">
          <div
            className="avatar avatar-md"
            style={{ backgroundColor: author?.avatarColor || "#8b5cf6" }}
          >
            {initials(author?.displayName)}
          </div>
        </div>
      )}
      <div className="message-content">
        {!compact && (
          <div className="message-meta">
            <span className="message-author">
              {author?.displayName ?? "…"}
            </span>
            <span className="message-time">
              {formatTime(message.createdAt)}
            </span>
          </div>
        )}
        <RichBody body={message.body} />
        {grouped.length > 0 && (
          <div className="reactions">
            {grouped.map(([emoji, { count, mine }]) => (
              <button
                key={emoji}
                onClick={() =>
                  void toggle.mutate({ messageId: message.id, emoji })
                }
                className={"reaction" + (mine ? " mine" : "")}
              >
                <span>{emoji}</span>
                <span className="reaction-count">{count}</span>
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
      await send.mutate({
        channelId,
        body: text,
        parentMessageId: parentId,
      });
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
      <div className="composer-inner">
        <textarea
          ref={textareaRef}
          value={body}
          onChange={(e) => setBody(e.target.value)}
          placeholder="Reply to thread…"
          className="composer-textarea"
          rows={1}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void submit();
            }
          }}
        />
        <button
          type="submit"
          disabled={!canSend}
          className="btn-send"
          aria-label="Send reply"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
            <path
              d="M5 12h14M13 6l6 6-6 6"
              stroke="currentColor"
              strokeWidth="2.2"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </div>
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

  if (typing.length === 0) return <div className="presence-bar" />;

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
    <div className="presence-bar">
      <span className="typing">
        <span className="typing-dots">
          <span className="typing-dot" />
          <span className="typing-dot" />
          <span className="typing-dot" />
        </span>
        <span>{label}</span>
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Composer
// ---------------------------------------------------------------------------

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
      className="composer"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <div className="composer-inner">
        <textarea
          ref={textareaRef}
          value={body}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          className="composer-textarea"
          rows={1}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void submit();
            }
          }}
        />
        <button
          type="submit"
          disabled={!canSend}
          className="btn-send"
          title="Send (Enter)"
          aria-label="Send message"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
            <path
              d="M5 12h14M13 6l6 6-6 6"
              stroke="currentColor"
              strokeWidth="2.2"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </div>
    </form>
  );
}

// ---------------------------------------------------------------------------
// Command palette — fuzzy switcher over channels + DMs + people
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

  // Reset selection when the filtered list changes so an arrow-key press
  // doesn't drop off the end of a shrunken list.
  useEffect(() => {
    setSel(0);
  }, [query, filtered.length]);

  async function activate(item: PaletteItem) {
    if (item.kind === "channel" || item.kind === "dm") {
      onSelectChannel(item.channel.id);
      return;
    }
    // A user with no existing DM — start one.
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

  // Split the filtered list into sections for visual clarity.
  const channelsOut = filtered.filter((i) => i.kind === "channel");
  const dmsOut = filtered.filter((i) => i.kind === "dm");
  const usersOut = filtered.filter((i) => i.kind === "user");

  // Selected item global index → check each section sequentially. A single
  // `sel` index over the flattened list keeps keyboard navigation simple.
  const flatIndex = (item: PaletteItem) =>
    filtered.findIndex((x) => x.id === item.id);

  return (
    <div
      className="palette-backdrop"
      onClick={onClose}
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
    >
      <div
        className="palette"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="palette-input-row">
          <svg
            className="palette-search-icon"
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
          >
            <circle
              cx="11"
              cy="11"
              r="7"
              stroke="currentColor"
              strokeWidth="2"
            />
            <path
              d="M21 21l-4.35-4.35"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
            />
          </svg>
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            placeholder="Jump to channel, DM, or person…"
            className="palette-input"
          />
          <kbd className="palette-kbd">Esc</kbd>
        </div>
        <div className="palette-list">
          {filtered.length === 0 ? (
            <div className="palette-empty">No matches.</div>
          ) : (
            <>
              {channelsOut.length > 0 && (
                <>
                  <div className="palette-section-label">Channels</div>
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
                  <div className="palette-section-label">Direct messages</div>
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
                  <div className="palette-section-label">People</div>
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
        <div className="palette-footer">
          <span className="palette-hint">
            <kbd className="palette-kbd">↑</kbd>
            <kbd className="palette-kbd">↓</kbd> navigate
          </span>
          <span className="palette-hint">
            <kbd className="palette-kbd">↵</kbd> select
          </span>
          <span className="palette-hint" style={{ marginLeft: "auto" }}>
            <kbd className="palette-kbd">⌘/</kbd> shortcuts
          </span>
        </div>
      </div>
    </div>
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
      className={"palette-item" + (selected ? " selected" : "")}
      onClick={onActivate}
      onMouseEnter={onHover}
      role="option"
      aria-selected={selected}
    >
      <div className="palette-item-icon">
        {item.kind === "channel" ? (
          <span>{item.channel.isPrivate ? "🔒" : "#"}</span>
        ) : item.kind === "dm" ? (
          <div
            className="avatar avatar-sm"
            style={{ backgroundColor: item.peer.avatarColor || "#8b5cf6" }}
          >
            {initials(item.peer.displayName)}
          </div>
        ) : (
          <div
            className="avatar avatar-sm"
            style={{ backgroundColor: item.user.avatarColor || "#8b5cf6" }}
          >
            {initials(item.user.displayName)}
          </div>
        )}
      </div>
      <div className="palette-item-label">{item.label}</div>
      <div className="palette-item-meta">
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
// Shortcut help overlay
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
    <div className="palette-backdrop" onClick={onClose}>
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ width: 420, maxHeight: "auto" }}
        role="dialog"
        aria-modal="true"
      >
        <div className="modal-title">Keyboard shortcuts</div>
        <div className="modal-subtitle">Fly around without touching a mouse.</div>
        <div className="shortcuts-grid">
          {rows.map((r) => (
            <React.Fragment key={r.label}>
              <div className="shortcut-label">{r.label}</div>
              <div className="shortcut-keys">
                {r.keys.map((k) => (
                  <kbd key={k} className="palette-kbd">
                    {k}
                  </kbd>
                ))}
              </div>
            </React.Fragment>
          ))}
        </div>
        <div className="modal-footer">
          <button className="btn-secondary" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Profile modal — read-only for others, editable for self.
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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

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
      // Also refresh the cached user in localStorage if editing self.
      if (isMe) {
        const next = {
          ...currentUser,
          displayName: name,
          email,
          avatarColor: color,
        };
        localStorage.setItem("statecraft_user", JSON.stringify(next));
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
      <div className="modal-backdrop" onClick={onClose}>
        <div className="modal" onClick={(e) => e.stopPropagation()}>
          <div className="modal-title">Profile</div>
          <div className="modal-subtitle">Loading…</div>
        </div>
      </div>
    );
  }

  const displayColor = editing ? color : user.avatarColor || "#8b5cf6";
  const displayName = editing ? name : user.displayName;
  const displayInitials = initials(displayName);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ width: 440 }}
        role="dialog"
        aria-modal="true"
      >
        <div className="detail-header">
          <div
            className="avatar"
            style={{ backgroundColor: displayColor }}
          >
            {displayInitials}
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div className="detail-name">
              {displayName}
              {isMe && (
                <span
                  style={{ color: "var(--text-dim)", marginLeft: 8, fontSize: 14 }}
                >
                  (you)
                </span>
              )}
            </div>
            <div className="detail-sub">{user.email}</div>
          </div>
        </div>

        {editing ? (
          <>
            <label className="field">
              <span className="field-label">Display name</span>
              <input
                className="input"
                value={name}
                onChange={(e) => setName(e.target.value)}
                autoFocus
              />
            </label>
            <label className="field">
              <span className="field-label">Email</span>
              <input
                className="input"
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            </label>
            <div className="detail-field">
              <div className="detail-field-label">Avatar color</div>
              <div className="color-swatches">
                {AVATAR_COLORS.map((c) => (
                  <button
                    key={c}
                    type="button"
                    className={
                      "color-swatch" +
                      (color.toLowerCase() === c.toLowerCase()
                        ? " selected"
                        : "")
                    }
                    style={{ background: c }}
                    onClick={() => setColor(c)}
                    aria-label={`Color ${c}`}
                  />
                ))}
              </div>
            </div>
            {err && <div className="login-error">{err}</div>}
          </>
        ) : (
          <>
            <div className="detail-field">
              <div className="detail-field-label">Email</div>
              <div className="detail-field-value">{user.email}</div>
            </div>
          </>
        )}

        <div className="modal-footer">
          {!isMe && !editing && (
            <button className="btn-secondary" onClick={() => void dm()}>
              Send message
            </button>
          )}
          {isMe && !editing && (
            <button
              className="btn-secondary"
              onClick={() => setEditing(true)}
            >
              Edit profile
            </button>
          )}
          {editing && (
            <>
              <button
                className="btn-secondary"
                onClick={() => {
                  setEditing(false);
                  setErr(null);
                }}
                disabled={busy}
              >
                Cancel
              </button>
              <button
                className="btn-primary"
                style={{ margin: 0, width: "auto", padding: "8px 18px" }}
                onClick={() => void save()}
                disabled={busy}
              >
                {busy ? "Saving…" : "Save"}
              </button>
            </>
          )}
          {!editing && (
            <button className="btn-secondary" onClick={onClose}>
              Close
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Channel details modal — info + edit for creator
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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  if (!channel) {
    return (
      <div className="modal-backdrop" onClick={onClose}>
        <div className="modal" onClick={(e) => e.stopPropagation()}>
          <div className="modal-title">Channel</div>
          <div className="modal-subtitle">Loading…</div>
        </div>
      </div>
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
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ width: 460 }}
        role="dialog"
        aria-modal="true"
      >
        <div className="detail-header">
          <div
            className="avatar"
            style={{
              background: channel.isPrivate
                ? "linear-gradient(135deg, #6366f1, #8b5cf6)"
                : "var(--surface-active)",
              color: channel.isPrivate ? "white" : "var(--text)",
            }}
          >
            {channel.isPrivate ? "🔒" : "#"}
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div className="detail-name">
              {channel.isPrivate ? "🔒 " : "# "}
              {channel.name}
            </div>
            <div className="detail-sub">
              {channel.isPrivate ? "Private channel" : "Public channel"}
            </div>
          </div>
        </div>

        {editing ? (
          <>
            <label className="field">
              <span className="field-label">Name</span>
              <input
                className="input"
                value={name}
                onChange={(e) => setName(e.target.value)}
                autoFocus
              />
            </label>
            <label className="field">
              <span className="field-label">Topic</span>
              <input
                className="input"
                value={topic}
                onChange={(e) => setTopic(e.target.value)}
                placeholder="What's this channel about?"
              />
            </label>
            <label
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "10px 12px",
                marginTop: 10,
                background: "var(--surface-raised)",
                border: "1px solid var(--border)",
                borderRadius: 10,
                cursor: "pointer",
                userSelect: "none",
              }}
            >
              <input
                type="checkbox"
                checked={isPrivate}
                onChange={(e) => setIsPrivate(e.target.checked)}
                style={{ accentColor: "var(--accent)" }}
              />
              <div>
                <div style={{ fontSize: 13.5, fontWeight: 500 }}>
                  Private channel
                </div>
                <div
                  style={{
                    fontSize: 12,
                    color: "var(--text-muted)",
                    marginTop: 2,
                  }}
                >
                  Only invited members can see or join.
                </div>
              </div>
            </label>
            {err && <div className="login-error">{err}</div>}
          </>
        ) : (
          <>
            {channel.topic ? (
              <div className="detail-field">
                <div className="detail-field-label">Topic</div>
                <div className="detail-field-value">{channel.topic}</div>
              </div>
            ) : (
              <div className="detail-field">
                <div className="detail-field-label">Topic</div>
                <div
                  className="detail-field-value"
                  style={{ color: "var(--text-dim)", fontStyle: "italic" }}
                >
                  No topic set.
                </div>
              </div>
            )}
            <div className="detail-field">
              <div className="detail-field-label">Created by</div>
              <div
                className="detail-field-value clickable"
                style={{ display: "flex", alignItems: "center", gap: 10 }}
                onClick={() =>
                  creator ? onOpenProfile(creator.id) : undefined
                }
              >
                <div
                  className="avatar avatar-sm"
                  style={{
                    backgroundColor: creator?.avatarColor || "#8b5cf6",
                  }}
                >
                  {initials(creator?.displayName)}
                </div>
                <span>{creator?.displayName ?? "Unknown"}</span>
              </div>
            </div>
            <div className="detail-field">
              <div className="detail-field-label">
                Members · {(memberships ?? []).length}
              </div>
              <div className="member-list">
                {(memberships ?? []).length === 0 ? (
                  <div
                    style={{
                      padding: "10px",
                      fontSize: 13,
                      color: "var(--text-dim)",
                    }}
                  >
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
          </>
        )}

        <div className="modal-footer">
          {canEdit && !editing && (
            <button
              className="btn-secondary"
              onClick={() => setEditing(true)}
            >
              Edit channel
            </button>
          )}
          {editing && (
            <>
              <button
                className="btn-secondary"
                onClick={() => {
                  setEditing(false);
                  setErr(null);
                }}
                disabled={busy}
              >
                Cancel
              </button>
              <button
                className="btn-primary"
                style={{ margin: 0, width: "auto", padding: "8px 18px" }}
                onClick={() => void save()}
                disabled={busy}
              >
                {busy ? "Saving…" : "Save"}
              </button>
            </>
          )}
          {!editing && (
            <button className="btn-secondary" onClick={onClose}>
              Close
            </button>
          )}
        </div>
      </div>
    </div>
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
    <button className="user-row" onClick={onClick}>
      <div
        className="avatar avatar-md"
        style={{ backgroundColor: user?.avatarColor || "#8b5cf6" }}
      >
        {initials(user?.displayName)}
      </div>
      <div className="user-row-meta">
        <div className="user-row-name">{user?.displayName ?? "…"}</div>
        <div className="user-row-email">{user?.email ?? ""}</div>
      </div>
      {role && (
        <span
          style={{
            fontSize: 10.5,
            fontWeight: 600,
            letterSpacing: "0.06em",
            textTransform: "uppercase",
            color: "var(--text-dim)",
            padding: "2px 8px",
            background: "var(--surface-raised)",
            border: "1px solid var(--border)",
            borderRadius: 4,
          }}
        >
          {role}
        </span>
      )}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

function EmptyState({ title, body }: { title: string; body: string }) {
  return (
    <div className="empty">
      <div className="empty-title">{title}</div>
      <div className="empty-body">{body}</div>
    </div>
  );
}

function CloseIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path
        d="M18 6L6 18M6 6l12 12"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}
