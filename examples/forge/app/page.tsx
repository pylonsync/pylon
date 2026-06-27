import React from "react";
import type { Metadata } from "@pylonsync/react";
import ForgeIsland from "./ForgeIsland";

export const metadata: Metadata = {
  title: "Pylon Forge",
  description: "Pylon Forge — Pylon example app",
};

// `app/page.tsx` → `/`. Forge's sync engine is client-only, so the page
// server-renders a shell and mounts the interactive UI as a client island.
export default function Page() {
  return <ForgeIsland />;
}
