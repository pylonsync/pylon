"use client";

import { configureClient, init, storageKey } from "@pylonsync/react";

// SSR-safe Pylon bootstrap, shared by both islands. Establishes a guest
// session BEFORE the collaborative UI mounts: doc creation and CRDT
// pushes need a signed-in caller. Same-origin under native SSR, so
// init() resolves window.location.origin — no baseUrl.
const APP_NAME = "pad";

export async function bootstrap(): Promise<string> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });

  const existing = window.localStorage.getItem(storageKey("token"));
  const existingUser = window.localStorage.getItem(storageKey("user"));
  if (existing && existingUser) return existingUser;

  try {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (!res.ok) throw new Error(`guest auth failed: ${res.status}`);
    const body = (await res.json()) as { token?: string; user_id?: string };
    if (body.token)
      window.localStorage.setItem(storageKey("token"), body.token);
    if (body.user_id)
      window.localStorage.setItem(storageKey("user"), body.user_id);
    window.localStorage.setItem(storageKey("isGuest"), "1");
    configureClient({ appName: APP_NAME });
    return body.user_id ?? "";
  } catch {
    return "";
  }
}

// Stable per-session identity for presence: a friendly name + color
// derived from the user id, so every window shows the same avatar for
// the same visitor.
const NAMES = [
  "Ada",
  "Grace",
  "Alan",
  "Edsger",
  "Barbara",
  "Donald",
  "Radia",
  "Linus",
  "Margaret",
  "Dennis",
  "Frances",
  "Ken",
];
const COLORS = [
  "#2563eb",
  "#db2777",
  "#16a34a",
  "#d97706",
  "#7c3aed",
  "#0891b2",
  "#dc2626",
  "#4d7c0f",
];

function hash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h);
}

export function identityFor(userId: string): { name: string; color: string } {
  const h = hash(userId || "anon");
  return {
    name: NAMES[h % NAMES.length],
    color: COLORS[h % COLORS.length],
  };
}
