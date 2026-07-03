"use client";

import React, { useEffect, useState } from "react";
import { bootstrap } from "../client/bootstrap";
import { Home } from "../client/Home";

// SSR renders a static shell; the live UI (guest session + sync engine)
// boots after hydration.
export default function HomeIsland() {
  const [userId, setUserId] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void bootstrap().then((uid) => {
      if (alive) setUserId(uid);
    });
    return () => {
      alive = false;
    };
  }, []);

  if (userId === null) {
    return (
      <div className="mx-auto max-w-2xl px-6 py-16 text-sm text-zinc-400">
        Connecting…
      </div>
    );
  }
  return <Home userId={userId} />;
}
