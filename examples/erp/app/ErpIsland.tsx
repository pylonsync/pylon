"use client";

import React, { useEffect, useState } from "react";
import { configureClient, init, storageKey } from "@pylonsync/react";
import { ErpApp } from "../client/ErpApp";

// Client island = the SSR-safe Pylon bootstrap. The ERP UI drives the sync
// engine (live queries, WebSocket fan-out, server-session tenant flips), none
// of which runs on the server — so we render a shell during SSR and boot here
// after hydration. Crucially we establish the guest session *before* mounting
// ErpApp: the demo's login flow calls an authenticated mutation (upsertUser)
// the moment the user clicks in, which 401s without a signed-in caller. Same
// origin under native SSR, so no baseUrl is needed — init() resolves
// window.location.origin.
const APP_NAME = "erp";

async function bootstrap(): Promise<void> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });

  // Already hold a guest/user token? Nothing to do.
  if (window.localStorage.getItem(storageKey("token"))) return;
  try {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (!res.ok) return;
    const body = (await res.json()) as { token?: string };
    // Only the token is established here. ErpApp reads storageKey("user") as a
    // JSON-encoded User object and writes it itself after its Login flow runs
    // upsertUser — so we must NOT pre-populate it.
    if (body.token) {
      window.localStorage.setItem(storageKey("token"), body.token);
    }
    window.localStorage.setItem(storageKey("isGuest"), "1");
    // Re-point the typed client now that we hold a session.
    configureClient({ appName: APP_NAME });
  } catch {
    // Pylon may not be reachable yet — ErpApp's hooks retry.
  }
}

export default function ErpIsland() {
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
  return <ErpApp />;
}
