// Pure chat helpers: names, colors, grouping, and labels. No Pylon imports,
// so tests/chat.test.ts runs them without a server and the server functions
// and the client share one copy.

export interface ChatMessage {
  id: string;
  channelId: string;
  authorId: string;
  text: string;
  /** Absent on an optimistic row until the server stamps it. */
  createdAt?: string | null;
}

export interface ChatMember {
  id: string;
  userId: string;
  name: string;
  hue: number;
}

export const MAX_MESSAGE_LENGTH = 4000;
export const MAX_NAME_LENGTH = 32;

/** Messages from one author closer together than this share one header. */
export const GROUP_GAP_MS = 5 * 60 * 1000;

const ADJECTIVES = [
  "Amber", "Brisk", "Cobalt", "Dusky", "Ember", "Fern", "Gilded", "Hazel",
  "Indigo", "Juniper", "Kestrel", "Lunar", "Maple", "Nimble", "Ochre", "Pewter",
  "Quiet", "Russet", "Saffron", "Tidal", "Umber", "Velvet", "Willow", "Zephyr",
];

const ANIMALS = [
  "Otter", "Heron", "Lynx", "Finch", "Marten", "Ibis", "Badger", "Wren",
  "Puffin", "Stoat", "Crane", "Hare", "Magpie", "Newt", "Osprey", "Plover",
  "Raven", "Seal", "Tern", "Vole", "Wolf", "Yak", "Gecko", "Moth",
];

/** FNV-1a: a small, stable string hash. The same id always maps to the same name. */
export function hashString(input: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < input.length; i++) {
    h ^= input.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** "Amber Otter" style display name for a guest, derived from the user id. */
export function guestName(userId: string): string {
  const h = hashString(userId);
  const adjective = ADJECTIVES[h % ADJECTIVES.length];
  const animal = ANIMALS[Math.floor(h / ADJECTIVES.length) % ANIMALS.length];
  return `${adjective} ${animal}`;
}

/** Avatar hue (0–359) derived from the user id. */
export function guestHue(userId: string): number {
  return hashString(`hue:${userId}`) % 360;
}

/** Trims, collapses whitespace, and validates a display name. Returns null when invalid. */
export function cleanName(raw: string): string | null {
  // \p{Cf} covers zero-width and bidi control characters, which could make a
  // name invisible or reorder the text around it.
  const name = raw.replace(/\p{Cf}/gu, "").replace(/\s+/g, " ").trim();
  if (name.length === 0 || name.length > MAX_NAME_LENGTH) return null;
  return name;
}

/** Up to two initials: "Maya Chen" → "MC", "theo" → "T". */
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  // Array.from splits by code point, so an emoji stays whole.
  const first = Array.from(parts[0])[0] ?? "";
  const last = parts.length > 1 ? (Array.from(parts[parts.length - 1])[0] ?? "") : "";
  return (first + last).toUpperCase();
}

/**
 * Chronological order. Rows the server has not stamped yet sort last. Equal
 * timestamps (Postgres stores whole seconds) fall back to the id, which is
 * time-ordered, so every client shows the same order.
 */
export function sortMessages<T extends ChatMessage>(messages: readonly T[]): T[] {
  return [...messages].sort((a, b) => {
    if (!a.createdAt && !b.createdAt) return 0;
    if (!a.createdAt) return 1;
    if (!b.createdAt) return -1;
    return a.createdAt.localeCompare(b.createdAt) || a.id.localeCompare(b.id);
  });
}

export interface MessageGroup<T extends ChatMessage = ChatMessage> {
  key: string;
  authorId: string;
  /** Local-day key ("2026-09-26") of the first message; null while pending. */
  day: string | null;
  messages: T[];
}

/** Local calendar day of an ISO timestamp, as "YYYY-MM-DD". */
export function dayKey(iso: string): string {
  const d = new Date(iso);
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${m}-${day}`;
}

/**
 * Groups sorted messages into runs by one author. A new group starts when the
 * author changes, the local day changes, or the gap exceeds GROUP_GAP_MS. A
 * pending row (no createdAt) joins the previous group when the author matches.
 */
export function groupMessages<T extends ChatMessage>(
  sorted: readonly T[],
  gapMs: number = GROUP_GAP_MS,
): MessageGroup<T>[] {
  const groups: MessageGroup<T>[] = [];
  let lastTime: number | null = null;
  for (const m of sorted) {
    const prev = groups[groups.length - 1];
    const time = m.createdAt ? Date.parse(m.createdAt) : null;
    const day = m.createdAt ? dayKey(m.createdAt) : null;
    const sameAuthor = prev?.authorId === m.authorId;
    const sameDay = day === null || prev?.day === null || prev?.day === day;
    const closeInTime = time === null || lastTime === null || time - lastTime <= gapMs;
    if (prev && sameAuthor && sameDay && closeInTime) {
      prev.messages.push(m);
    } else {
      groups.push({ key: m.id, authorId: m.authorId, day, messages: [m] });
    }
    if (time !== null) lastTime = time;
  }
  return groups;
}

/** "Today", "Yesterday", "Wednesday", or "Monday, September 21". */
export function dayLabel(iso: string, now: Date = new Date()): string {
  const d = new Date(iso);
  const startOf = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diffDays = Math.round((startOf(now) - startOf(d)) / 86_400_000);
  if (diffDays === 0) return "Today";
  if (diffDays === 1) return "Yesterday";
  if (diffDays > 1 && diffDays < 7) {
    return d.toLocaleDateString("en-US", { weekday: "long" });
  }
  return d.toLocaleDateString("en-US", {
    weekday: "long",
    month: "long",
    day: "numeric",
    ...(d.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
  });
}

/** "9:41 AM" in the viewer's locale. */
export function timeLabel(iso: string): string {
  return new Date(iso).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

/** "Maya is typing", "Maya and Theo are typing", "Maya, Theo, and 2 others are typing". */
export function typingLabel(names: readonly string[]): string {
  if (names.length === 0) return "";
  if (names.length === 1) return `${names[0]} is typing`;
  if (names.length === 2) return `${names[0]} and ${names[1]} are typing`;
  if (names.length === 3) return `${names[0]}, ${names[1]}, and ${names[2]} are typing`;
  return `${names[0]}, ${names[1]}, and ${names.length - 2} others are typing`;
}

/**
 * Messages from other people in a channel, newer than `lastSeen`. A channel the
 * viewer has no mark for counts as read, so a first visit shows no badges.
 */
export function unreadCount(
  messages: readonly ChatMessage[],
  channelId: string,
  lastSeen: string | undefined,
  selfId: string,
): number {
  if (!lastSeen) return 0;
  let n = 0;
  for (const m of messages) {
    if (m.channelId !== channelId || m.authorId === selfId || !m.createdAt) continue;
    if (m.createdAt > lastSeen) n++;
  }
  return n;
}
