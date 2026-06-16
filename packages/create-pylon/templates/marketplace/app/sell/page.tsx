import React from "react";
import { type Metadata } from "@pylonsync/react";
import { SellForm } from "../../client/SellForm";

export const metadata: Metadata = {
  title: "Sell an item · Pylon Market",
  description: "List something for sale in seconds — buyers' offers arrive live.",
};

export default function SellPage() {
  return (
    <div className="mx-auto max-w-xl space-y-6">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight">List an item</h1>
        <p className="text-sm text-muted-foreground">
          It goes live instantly. Offers land in your{" "}
          <a href="/me" className="underline">
            My Market
          </a>{" "}
          inbox in realtime.
        </p>
      </header>
      <SellForm />
    </div>
  );
}
