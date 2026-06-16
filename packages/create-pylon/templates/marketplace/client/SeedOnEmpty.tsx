"use client";

import React, { useEffect, useRef } from "react";
import { ensureDemoSeed, ensureReadSession } from "./market";

// First-run convenience: if the marketplace is empty, ensure the demo account
// + seed a dozen listings under it, then reload once so the server-rendered
// grid picks them up. Guarded by a session flag so it never loops. Real apps
// wouldn't ship this; it just makes `pylon dev` show something on first visit.
export function SeedOnEmpty({ count }: { count: number }) {
  const fired = useRef(false);
  useEffect(() => {
    if (count > 0 || fired.current) return;
    if (sessionStorage.getItem("market:seeded") === "1") return;
    fired.current = true;
    sessionStorage.setItem("market:seeded", "1");
    void (async () => {
      await ensureReadSession();
      await ensureDemoSeed();
      // The seed inserts listings owned by the demo user; reload so the SSR
      // grid renders them. The session flag prevents a reload loop.
      window.location.reload();
    })();
  }, [count]);
  return null;
}
