import React from "react";
import type { Metadata } from "@pylonsync/react";
import BenchIsland from "./BenchIsland";

export const metadata: Metadata = {
  title: "Pylon Bench",
  description: "Pylon Bench — Pylon example app",
};

// `app/page.tsx` → `/`. Bench is a realtime, sync-engine client app (it spawns
// WebWorker load generators), so the page server-renders a light shell and
// mounts the interactive UI as a client-only island (no sync engine on the
// server). One binary, one port — no separate Next.js app.
export default function Page() {
  return <BenchIsland />;
}
