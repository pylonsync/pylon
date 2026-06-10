import React from "react";
import type { Metadata } from "@pylonsync/react";
import ArenaIsland from "./ArenaIsland";

export const metadata: Metadata = {
  title: "Pylon Arena",
  description:
    "Mass-multiplayer dot world — realtime sync fanned out to every client from a single Pylon binary.",
};

// `app/page.tsx` → `/`. Arena is a realtime, sync-engine client app, so the
// page server-renders a light shell and mounts the interactive UI as a
// client-only island (no sync engine on the server). One binary, one port —
// no separate Next.js app.
export default function Page() {
  return <ArenaIsland />;
}
