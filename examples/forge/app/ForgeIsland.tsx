"use client";

import React, { useEffect, useState } from "react";
import { configureClient, init, storageKey } from "@pylonsync/react";
import { ForgeApp } from "../client/ForgeApp";

// Client island = the SSR-safe Pylon bootstrap. The Forge UI drives the sync
// engine (live queries, WebSocket fan-out, a Three.js render loop), none of
// which runs on the server — so we render a shell during SSR and boot here
// after hydration. The guest session is established BEFORE mounting ForgeApp
// so the first callFn (initTerrain, updateCursor) has a signed-in caller.
const APP_NAME = "forge";

function randomName() {
  const names = ["nova", "onyx", "echo", "lyra", "atlas", "rhea", "orion", "vega"];
  return `${names[Math.floor(Math.random() * names.length)]}_${Math.floor(Math.random() * 900 + 100)}`;
}

async function bootstrap(): Promise<{ userId: string }> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });

  const existingToken = window.localStorage.getItem(storageKey("token"));
  const existingUser = window.localStorage.getItem(storageKey("user"));
  if (existingToken && existingUser) {
    return { userId: existingUser };
  }

  const res = await fetch("/api/auth/guest", { method: "POST" });
  if (!res.ok) throw new Error(`guest auth failed: ${res.status}`);
  const body = (await res.json()) as { token?: string; user_id?: string };
  if (!body.token || !body.user_id) throw new Error("malformed guest response");
  window.localStorage.setItem(storageKey("token"), body.token);
  window.localStorage.setItem(storageKey("user"), body.user_id);
  window.localStorage.setItem(storageKey("isGuest"), "1");
  // Re-point the typed client now that we hold a session.
  configureClient({ appName: APP_NAME });
  return { userId: body.user_id };
}

export default function ForgeIsland() {
  const [state, setState] = useState<{ userId: string; myName: string } | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const myName = randomName();
    bootstrap()
      .then(({ userId }) => setState({ userId, myName }))
      .catch((e: unknown) => {
        const msg = e instanceof Error ? e.message : String(e);
        setError(msg);
      });
  }, []);

  if (error) {
    return (
      <div className="grid min-h-[70vh] place-items-center text-sm text-destructive">
        Failed to connect: {error}
      </div>
    );
  }

  if (!state) {
    return (
      <div className="grid min-h-screen place-items-center bg-[#0d0d12]">
        <div className="flex flex-col items-center gap-3">
          <div className="size-8 animate-spin rounded-full border-2 border-border border-t-primary" />
          <p className="text-sm text-muted-foreground">Connecting to the forge…</p>
        </div>
      </div>
    );
  }

  return <ForgeApp userId={state.userId} myName={state.myName} />;
}
