"use client";

// Shared client-side glue for Pylon Market: types, display helpers, and the
// email/password auth bootstrap. Same-origin under native SSR, so no baseUrl —
// init() resolves window.location.origin.
import { init, configureClient, callFn, storageKey } from "@pylonsync/react";

export const APP_NAME = "market";

// Prefilled demo account. Seeded on first load (registered if missing) so the
// login form is one click away from a working session — and so the browse
// catalog has a real owner you can sign in as to manage offers.
export const DEMO = {
  email: "demo@pylon.market",
  password: "pylondemo123",
  name: "Demo Seller",
} as const;

export interface Listing {
  id: string;
  sellerId: string;
  sellerName: string;
  title: string;
  description: string;
  price: number;
  category: string;
  condition: string;
  status: "active" | "sold";
  seed: string;
  createdAt: string;
}

export interface Offer {
  id: string;
  listingId: string;
  listingTitle: string;
  sellerId: string;
  buyerId: string;
  buyerName: string;
  amount: number;
  message?: string;
  status: "pending" | "accepted" | "declined";
  createdAt: string;
}

export interface Identity {
  userId: string;
  name: string;
}

// ---------------------------------------------------------------------------
// Display helpers (pure)
// ---------------------------------------------------------------------------

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** Deterministic gradient "photo" from a seed — no image hosting needed. */
export function gradient(seed: string): string {
  const h = hash(seed);
  const a = h % 360;
  const b = (a + 40 + ((h >> 3) % 90)) % 360;
  return `linear-gradient(135deg, hsl(${a} 68% 56%), hsl(${b} 72% 44%))`;
}

export function initials(title: string): string {
  return (
    title
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0]?.toUpperCase() ?? "")
      .join("") || "·"
  );
}

export function money(n: number): string {
  const v = Math.round(n * 100) / 100;
  return (
    "$" +
    v.toLocaleString(undefined, {
      minimumFractionDigits: v % 1 ? 2 : 0,
      maximumFractionDigits: 2,
    })
  );
}

export function timeAgo(iso: string): string {
  const s = (Date.now() - Date.parse(iso)) / 1000;
  if (!Number.isFinite(s)) return "";
  if (s < 60) return "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export function conditionLabel(c: string): string {
  return (
    { "like-new": "Like new", new: "New", good: "Good", fair: "Fair" }[c] ?? c
  );
}

// ---------------------------------------------------------------------------
// Auth — real email/password (no email verification)
// ---------------------------------------------------------------------------
//
// Storage slots:
//   storageKey("token")  — the bearer the sync engine sends on every request.
//                          A guest token for read connectivity, OR the signed-
//                          in user's token after login.
//   storageKey("userId") — set ONLY when a real user is signed in. Its
//                          presence is what "logged in" means.
//   market:displayName   — the signed-in user's name, cached for instant UI.

const TOKEN = () => storageKey("token");
const USER_ID = () => storageKey("userId");
const DISPLAY_NAME = "market:displayName";

let booted = false;

/** Boot the sync engine once. Same-origin, so init() resolves the origin. */
export function bootClient(): void {
  if (booted) return;
  booted = true;
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });
}

/**
 * Establish a guest session purely for READ connectivity — the public browse
 * grid + "just listed" ticker run live for signed-out visitors. Writes are
 * gated behind a real sign-in (see {@link signIn}), so this guest token never
 * authors a row. Idempotent: skipped once any token (guest or user) exists.
 */
export async function ensureReadSession(): Promise<void> {
  bootClient();
  if (localStorage.getItem(TOKEN())) return;
  try {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (res.ok) {
      const body = (await res.json()) as { token?: string };
      if (body.token) localStorage.setItem(TOKEN(), body.token);
      configureClient({ appName: APP_NAME });
    }
  } catch {
    // Server not reachable yet — the engine retries on connect.
  }
}

function applySession(token: string, userId: string, name: string): void {
  localStorage.setItem(TOKEN(), token);
  localStorage.setItem(USER_ID(), userId);
  localStorage.setItem(DISPLAY_NAME, name);
  configureClient({ appName: APP_NAME });
  // Notify in-tab + cross-tab listeners (MarketProvider) to re-read identity.
  window.dispatchEvent(new Event("pylon-auth-changed"));
}

type AuthResponse = { token: string; user_id: string };

export async function signIn(email: string, password: string): Promise<void> {
  const res = await fetch("/api/auth/password/login", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  const json = await res.json();
  if (!res.ok) throw new Error(json.error?.message ?? "Sign-in failed");
  const { token, user_id } = json as AuthResponse;
  // Login doesn't return displayName; default to the email handle and let
  // MarketProvider's live User query upgrade it.
  applySession(token, user_id, email.split("@")[0] ?? "you");
}

export async function signUp(
  email: string,
  password: string,
  name: string,
): Promise<void> {
  const res = await fetch("/api/auth/password/register", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      email,
      password,
      displayName: name || email.split("@")[0],
    }),
  });
  const json = await res.json();
  if (!res.ok) throw new Error(json.error?.message ?? "Sign-up failed");
  const { token, user_id } = json as AuthResponse;
  applySession(token, user_id, name || email.split("@")[0] || "you");
}

export async function signOut(): Promise<void> {
  localStorage.removeItem(TOKEN());
  localStorage.removeItem(USER_ID());
  localStorage.removeItem(DISPLAY_NAME);
  window.dispatchEvent(new Event("pylon-auth-changed"));
  // Restore read connectivity for the now-anonymous visitor.
  await ensureReadSession();
  window.dispatchEvent(new Event("pylon-auth-changed"));
}

export function readIdentity(): Identity | null {
  const userId = localStorage.getItem(USER_ID());
  if (!userId) return null;
  return { userId, name: localStorage.getItem(DISPLAY_NAME) ?? "you" };
}

/** Cache the freshest displayName (from the live User query) for instant UI. */
export function cacheDisplayName(name: string): void {
  if (name) localStorage.setItem(DISPLAY_NAME, name);
}

/**
 * Make sure the demo account exists and the catalog is seeded under it — so
 * the prefilled login is one click from a working session, and signing in as
 * the demo seller shows a real inventory to manage. Idempotent and best-effort:
 * any failure (already registered, offline) is swallowed.
 *
 * The demo token is used transiently to seed and is then discarded — it does
 * NOT become the visitor's active session. They stay anonymous until they
 * choose to sign in.
 */
export async function ensureDemoSeed(): Promise<void> {
  let token: string | undefined;
  try {
    const reg = await fetch("/api/auth/password/register", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        email: DEMO.email,
        password: DEMO.password,
        displayName: DEMO.name,
      }),
    });
    if (reg.ok) {
      token = ((await reg.json()) as AuthResponse).token;
    } else {
      // Already registered (or other) — log in to get a token for seeding.
      const login = await fetch("/api/auth/password/login", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email: DEMO.email, password: DEMO.password }),
      });
      if (login.ok) token = ((await login.json()) as AuthResponse).token;
    }
  } catch {
    return;
  }
  if (!token) return;
  try {
    // Seeds the catalog owned by the demo user (seedMarket stamps sellerId
    // from the caller). Idempotent server-side. Uses the demo token WITHOUT
    // persisting it as the active session.
    await callFn("seedMarket", {}, { token });
  } catch {
    // Catalog already seeded, or seeding raced — ignore.
  }
}
