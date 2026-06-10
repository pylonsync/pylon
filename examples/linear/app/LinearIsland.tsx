"use client";

import React, { useEffect, useState } from "react";
import { configureClient, init, storageKey } from "@pylonsync/react";
import { LinearApp } from "../client/LinearApp";

// Client island = the SSR-safe Pylon bootstrap. The Linear UI drives the sync
// engine (live queries, WebSocket fan-out), none of which runs on the server —
// so we render a shell during SSR and boot here after hydration. We establish
// the guest session *before* mounting LinearApp so its first API call (the
// in-app Login → guest → upsertUser flow, then mutations) has a signed-in
// caller. Same-origin under native SSR, so no baseUrl is needed — init()
// resolves window.location.origin.
//
// Note: LinearApp reads storageKey("user") as JSON of the full User object and
// shows its own <Login> screen until that exists. We therefore set only the
// guest token + isGuest here and let LinearApp's Login write the user JSON —
// writing a raw user_id string into storageKey("user") would corrupt its
// JSON.parse and is unnecessary.
const APP_NAME = "linear";

async function bootstrap(): Promise<void> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });

  // Already have a guest/user token? Nothing to do.
  if (window.localStorage.getItem(storageKey("token"))) return;
  try {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (!res.ok) return;
    const body = (await res.json()) as { token?: string; user_id?: string };
    if (body.token) window.localStorage.setItem(storageKey("token"), body.token);
    window.localStorage.setItem(storageKey("isGuest"), "1");
    // Re-point the typed client now that we hold a session.
    configureClient({ appName: APP_NAME });
  } catch {
    // Pylon may not be reachable yet — LinearApp's hooks retry.
  }
}

export default function LinearIsland() {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    void bootstrap().then(() => setReady(true));
  }, []);

  if (!ready) {
    return (
      <div className="grid min-h-[70vh] place-items-center text-sm text-muted-foreground">
        Loading…
      </div>
    );
  }
  return <LinearApp />;
}
