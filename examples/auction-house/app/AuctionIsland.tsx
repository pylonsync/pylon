"use client";

import React, { useEffect, useState } from "react";
import { configureClient, init, storageKey } from "@pylonsync/react";
import { AuctionApp } from "../client/AuctionApp";

// Client island = the SSR-safe Pylon bootstrap. The Auction House UI drives the
// sync engine (live queries, WebSocket fan-out), none of which runs on the
// server — so we render a shell during SSR and boot here after hydration.
// Crucially we establish the guest session *before* mounting AuctionApp: its
// first action (seedAuctionHouse) needs a signed-in caller, so the app must
// not mount until the session exists. Same-origin under native SSR, so no
// baseUrl is needed — init() resolves window.location.origin.
const APP_NAME = "auction-house";

async function bootstrap(): Promise<void> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });

  // Already have a guest/user token? Nothing to do.
  if (window.localStorage.getItem(storageKey("token"))) return;
  try {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (!res.ok) return;
    const body = (await res.json()) as { token?: string; user_id?: string };
    // The auction-house auth helper (client/lib/auth.tsx) reads
    // storageKey("token") + storageKey("userId") + storageKey("isGuest").
    if (body.token) window.localStorage.setItem(storageKey("token"), body.token);
    if (body.user_id)
      window.localStorage.setItem(storageKey("userId"), body.user_id);
    window.localStorage.setItem(storageKey("isGuest"), "1");
    // Re-point the typed client now that we hold a session.
    configureClient({ appName: APP_NAME });
  } catch {
    // Pylon may not be reachable yet — AuctionApp's hooks retry.
  }
}

export default function AuctionIsland() {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    void bootstrap().then(() => setReady(true));
  }, []);

  if (!ready) {
    return (
      <div className="grid min-h-[70vh] place-items-center text-sm text-muted-foreground">
        Loading the auction house…
      </div>
    );
  }
  return <AuctionApp />;
}
