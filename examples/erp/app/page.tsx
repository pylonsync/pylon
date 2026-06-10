import React from "react";
import type { Metadata } from "@pylonsync/react";
import ErpIsland from "./ErpIsland";

export const metadata: Metadata = {
  title: "Pylon ERP",
  description: "Pylon ERP — Pylon example app",
};

// `app/page.tsx` → `/`. ERP is a sync-engine client app, so the page
// server-renders a light shell and mounts the interactive UI as a
// client-only island (no sync engine on the server). One binary, one port —
// no separate Next.js app.
export default function Page() {
  return <ErpIsland />;
}
