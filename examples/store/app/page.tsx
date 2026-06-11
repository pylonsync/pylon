import React from "react";
import type { Metadata } from "@pylonsync/react";
import StoreIsland from "./StoreIsland";

export const metadata: Metadata = {
  title: "Pylon Store",
  description: "Pylon Store — faceted search, live cart, and order tracking",
};

// `app/page.tsx` → `/`. Store is a sync-engine client app, so the page
// server-renders a light shell and mounts the interactive UI as a client-only
// island. One binary, one port — no separate Next.js / Vite process.
export default function Page() {
  return <StoreIsland />;
}
