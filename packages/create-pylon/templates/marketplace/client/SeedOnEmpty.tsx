"use client";

import React, { useEffect, useRef } from "react";
import { ensureDemoSeed, ensureReadSession } from "./market";

// First-run convenience: if the marketplace is empty, ensure the demo account
// + seed a dozen listings under it, then reload once so the server-rendered
// grid picks them up. A short retry window prevents reload loops while still
// recovering when the dev database is reset but browser storage survives.
export function SeedOnEmpty({ count }: { count: number }) {
  const fired = useRef(false);
  useEffect(() => {
    if (count > 0 || fired.current) return;
    const lastAttempt = Number(sessionStorage.getItem("market:seed-attempted-at"));
    if (Number.isFinite(lastAttempt) && Date.now() - lastAttempt < 10_000) return;
    fired.current = true;
    sessionStorage.setItem("market:seed-attempted-at", String(Date.now()));
    void (async () => {
      await ensureReadSession();
      await ensureDemoSeed({ force: true });
      // The seed inserts listings owned by the demo user; reload so the SSR
      // grid renders them.
      window.location.reload();
    })();
  }, [count]);
  return null;
}
