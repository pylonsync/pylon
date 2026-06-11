"use client";

import React, { useEffect, useState } from "react";
import { configureClient, init, storageKey } from "@pylonsync/react";
import { StoreApp } from "../client/StoreApp";

// Client island — boots the Pylon sync engine + guest session before
// mounting StoreApp. The store allows unauthenticated browsing, so we
// establish a guest session on every cold load. The actual login / register
// flow in AuthDialog upgrades it to a real user session.
//
// Same-origin under native SSR, so no explicit baseUrl is needed — init()
// resolves window.location.origin.
const APP_NAME = "store";

async function bootstrap(): Promise<void> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });

  // Already hold a guest/user token? Nothing to do.
  if (window.localStorage.getItem(storageKey("token"))) return;
  try {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (!res.ok) return;
    const body = (await res.json()) as { token?: string; user_id?: string };
    if (body.token) {
      window.localStorage.setItem(storageKey("token"), body.token);
    }
    if (body.user_id) {
      window.localStorage.setItem(storageKey("userId"), body.user_id);
    }
    window.localStorage.setItem(storageKey("isGuest"), "1");
    configureClient({ appName: APP_NAME });
  } catch {
    // Pylon may not be reachable yet — StoreApp's hooks retry.
  }
}

export default function StoreIsland() {
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
  return <StoreApp />;
}
