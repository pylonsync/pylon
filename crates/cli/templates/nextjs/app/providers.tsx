"use client";

import { useEffect } from "react";
import { configureClient, useSyncStatus } from "@pylonsync/react";

// Configure the client-side Pylon SDK. baseUrl is empty so requests
// go same-origin and the Next.js proxy in next.config.js forwards
// them to PYLON_TARGET. The session cookie rides along automatically.
export function Providers({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    configureClient({ baseUrl: "" });
  }, []);
  return (
    <>
      {children}
      <ConnectionIndicator />
    </>
  );
}

/**
 * Lightweight banner that surfaces sync engine connection state. Stays
 * invisible while the WS is connected (or before the engine is
 * initialized) and slides in only when the user needs to know
 * something — typically during cold starts after a Fly autostop.
 *
 * Drop this anywhere; it auto-positions in the bottom-right and
 * doesn't block clicks. Customize the copy / styling to taste.
 */
function ConnectionIndicator() {
  const status = useSyncStatus();
  if (status === "connected" || status === "offline") return null;
  return (
    <div className="fixed bottom-4 right-4 z-50 pointer-events-none">
      <div className="rounded-md bg-foreground/90 text-background text-xs font-medium px-3 py-2 shadow-lg backdrop-blur">
        {status === "connecting" ? "Connecting…" : "Reconnecting…"}
      </div>
    </div>
  );
}
