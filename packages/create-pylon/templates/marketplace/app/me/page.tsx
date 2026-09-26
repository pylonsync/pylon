import React from "react";
import { type Metadata } from "@pylonsync/react";
import { MyMarket } from "../../client/MyMarket";

export const metadata: Metadata = {
  title: "Your market | Reprise",
  description: "Your listings, offers, and saved items.",
  robots: "noindex", // personal dashboard; keep it out of search
};

// The dashboard is three live queries scoped to the signed-in user, so the
// whole page is one client island.
export default function MePage() {
  return <MyMarket />;
}
