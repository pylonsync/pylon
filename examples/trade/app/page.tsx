import React from "react";
import type { Metadata } from "@pylonsync/react";
import TradeIsland from "./TradeIsland";

export const metadata: Metadata = {
  title: "Pylon Trade",
  description: "Pylon Trade — Pylon example app",
};

// `app/page.tsx` → `/`. Trade is a realtime, sync-engine client app, so the
// page server-renders a light shell and mounts the interactive UI as a
// client-only island (no sync engine on the server). One binary, one port —
// no separate Next.js app.
export default function Page() {
  return <TradeIsland />;
}
