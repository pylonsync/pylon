"use client";

import React, { useEffect, useState } from "react";
import { configureClient, init } from "@pylonsync/react";
import { TodoApp } from "../client/TodoApp";

// Client island = the SSR-safe Pylon bootstrap. The Todo UI drives the sync
// engine (live queries, optimistic mutations), none of which runs on the
// server — so we render a shell during SSR and boot here after hydration.
// Same-origin under native SSR, so no baseUrl is needed — init() resolves
// window.location.origin.
//
// Unlike the guest-session demos (arena, world3d), Todo ships a REAL
// email/password login flow: TodoApp renders its own <Login /> screen and
// reads storageKey("token") + storageKey("userId") from a real password
// auth (/api/auth/password/*). We must NOT mint a guest session here — that
// would write into the same token/userId slots the real login uses, hide the
// login screen, and point reads at a guest user who owns no todos. The island
// only initializes the client; the user signs in through TodoApp.
const APP_NAME = "todo-app";

async function bootstrap(): Promise<void> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });
}

export default function TodoIsland() {
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
  return <TodoApp />;
}
