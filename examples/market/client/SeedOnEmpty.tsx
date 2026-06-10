"use client";

import React, { useEffect, useRef } from "react";
import { callFn } from "@pylonsync/react";
import { ensureIdentity } from "./market";

// First-run convenience: if the marketplace is empty, seed a dozen demo
// listings, then reload once so the server-rendered grid picks them up.
// Guarded by a session flag so it never loops. Real apps wouldn't ship this;
// it just makes `pylon dev` show something on the very first visit.
export function SeedOnEmpty({ count }: { count: number }) {
  const fired = useRef(false);
  useEffect(() => {
    if (count > 0 || fired.current) return;
    if (sessionStorage.getItem("market:seeded") === "1") return;
    fired.current = true;
    sessionStorage.setItem("market:seeded", "1");
    void (async () => {
      await ensureIdentity();
      try {
        const res = await callFn<{ seeded: number }>("seedMarket", {});
        if (res.seeded > 0) window.location.reload();
      } catch {
        /* ignore — page still works empty */
      }
    })();
  }, [count]);
  return null;
}
