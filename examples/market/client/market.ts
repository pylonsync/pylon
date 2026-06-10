"use client";

// Shared client-side glue for Pylon Market: types, display helpers, and the
// guest-identity bootstrap. Same-origin under native SSR, so no baseUrl —
// init() resolves window.location.origin.
import { init, configureClient, storageKey } from "@pylonsync/react";

export const APP_NAME = "market";

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
// Guest identity
// ---------------------------------------------------------------------------

const ADJ = [
  "swift", "amber", "cosmic", "ivory", "slate", "maple", "violet", "copper",
  "jade", "cobalt", "rust", "fern",
];
const NOUN = [
  "otter", "lynx", "wren", "heron", "fox", "sparrow", "marten", "finch",
  "ibis", "stoat", "vole", "shrike",
];

function pick<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)];
}

let _identity: Identity | null = null;

/**
 * Boot the sync engine + a guest session, and mint a stable display handle
 * (stored in localStorage) so listings/offers have a human name. Idempotent.
 */
export async function ensureIdentity(): Promise<Identity> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });

  if (!localStorage.getItem(storageKey("token"))) {
    try {
      const res = await fetch("/api/auth/guest", { method: "POST" });
      if (res.ok) {
        const body = (await res.json()) as { token?: string; user_id?: string };
        if (body.token) localStorage.setItem(storageKey("token"), body.token);
        if (body.user_id) localStorage.setItem(storageKey("user"), body.user_id);
        localStorage.setItem(storageKey("isGuest"), "1");
        configureClient({ appName: APP_NAME });
      }
    } catch {
      // Pylon not reachable yet — callers retry through the sync engine.
    }
  }

  const userId = localStorage.getItem(storageKey("user")) ?? "";
  let name = localStorage.getItem("market:name");
  if (!name) {
    name = `${pick(ADJ)}-${pick(NOUN)}`;
    localStorage.setItem("market:name", name);
  }
  _identity = { userId, name };
  return _identity;
}

export function currentIdentity(): Identity | null {
  return _identity;
}
