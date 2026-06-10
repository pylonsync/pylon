import React from "react";
import type { Metadata } from "@pylonsync/react";
import AuctionIsland from "./AuctionIsland";

export const metadata: Metadata = {
  title: "Pylon Auctions",
  description: "Pylon Auctions — Pylon example app",
};

// `app/page.tsx` → `/`. Auction House is a realtime, sync-engine client app,
// so the page server-renders a light shell and mounts the interactive UI as a
// client-only island (no sync engine on the server). One binary, one port —
// no separate Next.js app.
export default function Page() {
  return <AuctionIsland />;
}
