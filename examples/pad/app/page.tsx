import React from "react";
import type { Metadata } from "@pylonsync/react";
import HomeIsland from "./HomeIsland";

export const metadata: Metadata = {
  title: "Pad — collaborative markdown on Pylon",
  description:
    "A collaborative markdown editor in one entity and two pages. Open a doc in two windows and watch keystrokes merge live — CRDT sync from a single Pylon binary.",
};

export default function Page() {
  return <HomeIsland />;
}

