import { useEffect, useState } from "react";
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

  if (!me) {
    return <Login onSignedIn={(next) => setMe(next)} />;
  }

  return <Dashboard me={me} onSignOut={() => { clearToken(); setMe(null); }} />;
}
