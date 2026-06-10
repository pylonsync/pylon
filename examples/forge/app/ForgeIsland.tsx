"use client";

import React, { useEffect, useState } from "react";
import { configureClient, init, storageKey } from "@pylonsync/react";
import { ForgeApp } from "../client/ForgeApp";

// Client island = the SSR-safe Pylon bootstrap. The Forge UI drives the sync
// engine (live queries, WebSocket fan-out, a Three.js render loop), none of
// which runs on the server — so we render a shell during SSR and boot here
// after hydration. Crucially we establish the guest session *before* mounting
// ForgeApp: its first action (spawnPrim/updateCursor) needs a signed-in
// caller, so the app must not mount until the session exists. Same-origin
// under native SSR, so no baseUrl is needed — init() resolves
// window.location.origin.
const APP_NAME = "forge";

async function bootstrap(): Promise<void> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });

  // Already have a guest/user token? Nothing to do.
  if (window.localStorage.getItem(storageKey("token"))) return;
  try {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (!res.ok) return;
    const body = (await res.json()) as { token?: string; user_id?: string };
    // ForgeApp reads storageKey("token") + storageKey("user").
    if (body.token) window.localStorage.setItem(storageKey("token"), body.token);
    if (body.user_id) window.localStorage.setItem(storageKey("user"), body.user_id);
    window.localStorage.setItem(storageKey("isGuest"), "1");
    // Re-point the typed client now that we hold a session.
    configureClient({ appName: APP_NAME });
  } catch {
    // Pylon may not be reachable yet — ForgeApp's hooks retry.
  }
}

export default function ForgeIsland() {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    void bootstrap().then(() => setReady(true));
  }, []);

  if (!ready) {
    return (
      <div className="grid min-h-[70vh] place-items-center text-sm text-muted-foreground">
        Connecting to the forge…
      </div>
    );
  }
  return <ForgeApp />;
}
