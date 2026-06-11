"use client";

import React, { useEffect, useState } from "react";
import { configureClient, init, storageKey } from "@pylonsync/react";
import { ArenaApp } from "../client/ArenaApp";

// Client island = the SSR-safe Pylon bootstrap. The Arena UI drives the sync
// engine (live queries, WebSocket fan-out, a canvas loop), none of which runs
// on the server — so we render a shell during SSR and boot here after
// hydration. Crucially we establish the guest session *before* mounting
// ArenaApp: its first action (spawnDot) needs a signed-in caller, so the app
// must not mount until the session exists. Same-origin under native SSR, so
// no baseUrl is needed — init() resolves window.location.origin.
const APP_NAME = "arena";

async function bootstrap(): Promise<string> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });

  // Already have a guest/user token? Reuse it.
  const existing = window.localStorage.getItem(storageKey("token"));
  const existingUser = window.localStorage.getItem(storageKey("user"));
  if (existing && existingUser) return existingUser;

  try {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (!res.ok) throw new Error(`guest auth failed: ${res.status}`);
    const body = (await res.json()) as { token?: string; user_id?: string };
    if (body.token) window.localStorage.setItem(storageKey("token"), body.token);
    if (body.user_id) window.localStorage.setItem(storageKey("user"), body.user_id);
    window.localStorage.setItem(storageKey("isGuest"), "1");
    // Re-point the typed client now that we hold a session.
    configureClient({ appName: APP_NAME });
    return body.user_id ?? "";
  } catch {
    // Pylon may not be reachable yet — ArenaApp's hooks retry via spawnDot.
    return "";
  }
}

export default function ArenaIsland() {
  const [userId, setUserId] = useState<string | null>(null);

  useEffect(() => {
    void bootstrap().then((id) => setUserId(id || ""));
  }, []);

  if (userId === null) {
    return (
      <div className="grid min-h-screen place-items-center text-sm text-muted-foreground">
        Connecting to the arena…
      </div>
    );
  }

  return <ArenaApp userId={userId} />;
}
