"use client";

/**
 * Client-side Pylon initializer for the linear example.
 *
 * Boots the sync engine at module load and ensures a guest session
 * exists before rendering, so child components can call db.useQuery
 * without race conditions on first paint.
 */
import { useEffect, useState } from "react";
import { configureClient, init, storageKey } from "@pylonsync/react";

const BASE_URL = process.env.NEXT_PUBLIC_PYLON_URL ?? "http://localhost:4321";
const WS_URL = BASE_URL.startsWith("https://")
  ? `${BASE_URL.replace(/^https:/, "wss:").replace(/\/$/, "")}:4322`
  : undefined;

let initialized = false;

function initOnce() {
  if (initialized) return;
  initialized = true;
  init({ baseUrl: BASE_URL, appName: "linear", wsUrl: WS_URL });
  configureClient({ baseUrl: BASE_URL, appName: "linear" });
}

async function ensureGuestSession(): Promise<void> {
  if (typeof window === "undefined") return;
  if (window.localStorage.getItem(storageKey("token"))) return;
  try {
    const res = await fetch(`${BASE_URL}/api/auth/guest`, { method: "POST" });
    if (!res.ok) return;
    const body = (await res.json()) as { token?: string; user_id?: string };
    if (body.token) window.localStorage.setItem(storageKey("token"), body.token);
    if (body.user_id)
      window.localStorage.setItem(storageKey("userId"), body.user_id);
    window.localStorage.setItem(storageKey("isGuest"), "1");
    configureClient({ baseUrl: BASE_URL, appName: "linear" });
  } catch {
    // Pylon may not be running yet — child useQuery hooks will retry.
  }
}

export function PylonProvider({ children }: { children: React.ReactNode }) {
  const [ready, setReady] = useState(false);
  // useState initializer runs synchronously during render, so init()
  // lands BEFORE any child useQuery hook can race ahead and create a
  // fallback engine pointed at the wrong baseUrl. Earlier we shipped a
  // bug where SyncProvider initialized in useEffect and child queries
  // briefly hit localhost:4321 in production.
  useState(() => {
    initOnce();
    return true;
  });
  useEffect(() => {
    void ensureGuestSession().then(() => setReady(true));
  }, []);
  if (!ready) return null;
  return <>{children}</>;
}
