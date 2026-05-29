// Client-side Pylon initializer for the chat example.
//
// Boots the sync engine at module load and ensures a guest session
// exists before rendering, so child components can call db.useQuery
// without race conditions on first paint.

import { useEffect, useState } from "react";
import { configureClient, init, storageKey } from "@pylonsync/react";

let initialized = false;

function initOnce(baseUrl: string) {
  if (initialized) return;
  initialized = true;
  init({ baseUrl, appName: "chat" });
  configureClient({ baseUrl, appName: "chat" });
}

async function ensureGuestSession(baseUrl: string): Promise<void> {
  if (typeof window === "undefined") return;
  if (window.localStorage.getItem(storageKey("token"))) return;
  try {
    const res = await fetch(`${baseUrl}/api/auth/guest`, { method: "POST" });
    if (!res.ok) return;
    const body = (await res.json()) as { token?: string; user_id?: string };
    if (body.token) window.localStorage.setItem(storageKey("token"), body.token);
    if (body.user_id)
      window.localStorage.setItem(storageKey("userId"), body.user_id);
    window.localStorage.setItem(storageKey("isGuest"), "1");
    configureClient({ baseUrl, appName: "chat" });
  } catch {
    // Pylon may not be running yet — child useQuery hooks will retry.
  }
}

export function PylonProvider({
  baseUrl,
  children,
}: {
  baseUrl: string;
  children: React.ReactNode;
}) {
  const [ready, setReady] = useState(false);
  // useState initializer runs synchronously during render, so init()
  // lands BEFORE any child useQuery hook can race ahead and create a
  // fallback engine pointed at the wrong baseUrl.
  useState(() => {
    initOnce(baseUrl);
    return true;
  });
  useEffect(() => {
    void ensureGuestSession(baseUrl).then(() => setReady(true));
  }, [baseUrl]);
  if (!ready) return null;
  return <>{children}</>;
}
