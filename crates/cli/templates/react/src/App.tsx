import { useEffect, useState } from "react";
import { useSyncStatus } from "@pylonsync/react";
import { pylonJson, getToken, clearToken, type Me } from "@/lib/pylon";
import { Login } from "./Login";
import { Dashboard } from "./Dashboard";

export function App() {
  const [me, setMe] = useState<Me | null | undefined>(undefined);

  // Resolve current session on first paint.
  useEffect(() => {
    async function load() {
      if (!getToken()) {
        setMe(null);
        return;
      }
      try {
        const data = await pylonJson<Me>("/api/auth/me");
        setMe(data.user_id ? data : null);
      } catch {
        clearToken();
        setMe(null);
      }
    }
    load();
  }, []);

  if (me === undefined) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <>
      {me ? (
        <Dashboard me={me} onSignOut={() => { clearToken(); setMe(null); }} />
      ) : (
        <Login onSignedIn={(next) => setMe(next)} />
      )}
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
