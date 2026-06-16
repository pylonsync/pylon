import React from "react";
import { type Metadata } from "@pylonsync/react";
import { MyMarket } from "../../client/MyMarket";

export const metadata: Metadata = {
  title: "My Market · Pylon Market",
  description: "Your listings and offers.",
  robots: "noindex", // personal dashboard — keep it out of search
};

// Fully interactive dashboard (three live queries scoped to you), so the
// whole page is the client island.
export default function MePage() {
  return <MyMarket />;
}
