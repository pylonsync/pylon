// The social model: usernames, counts, time labels, and which posts a
// person sees. Pure — no React, no `db` — so the rules are testable without a
// server, and the components stay thin.

export interface Profile {
  id: string;
  userId: string;
  username: string;
  displayName: string;
  bio?: string | null;
  avatarUrl?: string | null;
  createdAt?: string | null;
}

export interface Post {
  id: string;
  authorId: string;
  imageUrl: string;
  caption?: string | null;
  /** "square" (1:1) or "portrait" (4:5). The feed crops to this shape. */
  shape?: string | null;
  createdAt: string;
}

export interface Like {
  id: string;
  userId: string;
  postId: string;
}

export interface Comment {
  id: string;
  postId: string;
  authorId: string;
  text: string;
  createdAt: string;
}

export interface Follow {
  id: string;
  followerId: string;
  followingId: string;
}

export interface Save {
  id: string;
  userId: string;
  postId: string;
}

// ---------------------------------------------------------------------------
// Usernames
// ---------------------------------------------------------------------------

export const USERNAME_MIN = 3;
export const USERNAME_MAX = 30;
export const CAPTION_MAX = 2200;
export const COMMENT_MAX = 500;
export const BIO_MAX = 150;
export const DISPLAY_NAME_MAX = 40;

/** Lowercase, trimmed, a leading @ dropped. Does not validate. */
export function normalizeUsername(raw: string): string {
  return raw.trim().replace(/^@+/, "").toLowerCase();
}

/**
 * Why a username is refused, or null when it is fine: 3 to 30 characters of
 * letters, digits, periods, and underscores, not starting or ending with a
 * period, and no two periods in a row.
 */
export function usernameProblem(username: string): string | null {
  if (username.length < USERNAME_MIN) return `Use at least ${USERNAME_MIN} characters.`;
  if (username.length > USERNAME_MAX) return `Use at most ${USERNAME_MAX} characters.`;
  if (!/^[a-z0-9._]+$/.test(username)) return "Use letters, numbers, periods, and underscores only.";
  if (username.startsWith(".") || username.endsWith(".")) return "A username cannot start or end with a period.";
  if (username.includes("..")) return "A username cannot have two periods in a row.";
  return null;
}

/** A stable starting username for a new account, from its user id. */
export function starterUsername(userId: string): string {
  let hash = 2166136261;
  for (let i = 0; i < userId.length; i++) {
    hash ^= userId.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return `visitor_${(hash >>> 0).toString(36).slice(0, 6)}`;
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

/** 999 → "999", 1204 → "1,204", 12_500 → "12.5K", 3_400_000 → "3.4M". */
export function formatCount(n: number): string {
  if (n < 10_000) return n.toLocaleString("en-US");
  if (n < 1_000_000) return `${trimZero((n / 1000).toFixed(1))}K`;
  return `${trimZero((n / 1_000_000).toFixed(1))}M`;
}

function trimZero(s: string): string {
  return s.endsWith(".0") ? s.slice(0, -2) : s;
}

/** "1 like", "2 likes". */
export function plural(n: number, one: string, many = `${one}s`): string {
  return `${formatCount(n)} ${n === 1 ? one : many}`;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const WEEK = 7 * DAY;

/** Short age: "now", "5m", "3h", "4d", "2w", then a date. */
export function timeAgo(iso: string, now: number = Date.now()): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const age = Math.max(0, now - then);
  if (age < MINUTE) return "now";
  if (age < HOUR) return `${Math.floor(age / MINUTE)}m`;
  if (age < DAY) return `${Math.floor(age / HOUR)}h`;
  if (age < WEEK) return `${Math.floor(age / DAY)}d`;
  if (age < 5 * WEEK) return `${Math.floor(age / WEEK)}w`;
  const date = new Date(then);
  const sameYear = date.getUTCFullYear() === new Date(now).getUTCFullYear();
  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    ...(sameYear ? {} : { year: "numeric" }),
    timeZone: "UTC",
  });
}

/**
 * A caption or comment split into text and @mentions, so a mention can link to
 * the profile. Only names that pass `usernameProblem` become links.
 */
export type TextPart = { kind: "text"; text: string } | { kind: "mention"; username: string };

export function splitMentions(text: string): TextPart[] {
  const parts: TextPart[] = [];
  const pattern = /@([a-z0-9._]{3,30})/gi;
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    const username = match[1].toLowerCase().replace(/\.+$/, "");
    if (usernameProblem(username)) continue;
    const start = match.index ?? 0;
    if (start > last) parts.push({ kind: "text", text: text.slice(last, start) });
    parts.push({ kind: "mention", username });
    last = start + 1 + username.length;
  }
  if (last < text.length) parts.push({ kind: "text", text: text.slice(last) });
  return parts;
}

// ---------------------------------------------------------------------------
// Who sees what
// ---------------------------------------------------------------------------

export function newestFirst<T extends { createdAt: string }>(rows: T[]): T[] {
  return [...rows].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}

/** The user ids `userId` follows. */
export function followingOf(follows: Follow[], userId: string | null | undefined): Set<string> {
  const out = new Set<string>();
  if (!userId) return out;
  for (const f of follows) if (f.followerId === userId) out.add(f.followingId);
  return out;
}

/**
 * The home feed: posts by the people you follow and your own, newest first.
 * Someone who follows nobody yet sees every post, so a new account does not
 * open on an empty page.
 */
export function homeFeed(posts: Post[], following: Set<string>, me: string | null | undefined): { posts: Post[]; everyone: boolean } {
  if (following.size === 0) return { posts: newestFirst(posts), everyone: true };
  return {
    posts: newestFirst(posts.filter((p) => following.has(p.authorId) || p.authorId === me)),
    everyone: false,
  };
}

/** Accounts to suggest: not you, not already followed, most followed first. */
export function suggestions(
  profiles: Profile[],
  follows: Follow[],
  following: Set<string>,
  me: string | null | undefined,
  limit = 5,
): Profile[] {
  const followers = new Map<string, number>();
  for (const f of follows) followers.set(f.followingId, (followers.get(f.followingId) ?? 0) + 1);
  return profiles
    .filter((p) => p.userId !== me && !following.has(p.userId))
    .sort((a, b) => (followers.get(b.userId) ?? 0) - (followers.get(a.userId) ?? 0) || a.username.localeCompare(b.username))
    .slice(0, limit);
}

/** People who posted in the last `hours`, most recent first, one entry each. */
export function recentPosters(posts: Post[], now: number = Date.now(), hours = 48): string[] {
  const cutoff = now - hours * HOUR;
  const seen = new Set<string>();
  const out: string[] = [];
  for (const post of newestFirst(posts)) {
    if (Date.parse(post.createdAt) < cutoff) break;
    if (seen.has(post.authorId)) continue;
    seen.add(post.authorId);
    out.push(post.authorId);
  }
  return out;
}

export function countBy<T>(rows: T[], key: (row: T) => string): Map<string, number> {
  const out = new Map<string, number>();
  for (const row of rows) {
    const k = key(row);
    out.set(k, (out.get(k) ?? 0) + 1);
  }
  return out;
}

export function groupBy<T>(rows: T[], key: (row: T) => string): Map<string, T[]> {
  const out = new Map<string, T[]>();
  for (const row of rows) {
    const k = key(row);
    const list = out.get(k);
    if (list) list.push(row);
    else out.set(k, [row]);
  }
  return out;
}

/** A photo URL a post may use: an uploaded file, a file in public/, or https. */
export function isPostImageUrl(url: string): boolean {
  if (url.startsWith("/api/files/") || url.startsWith("/images/")) return !url.includes("..");
  try {
    return new URL(url).protocol === "https:";
  } catch {
    return false;
  }
}
