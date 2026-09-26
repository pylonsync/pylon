import React from "react";
import { type Metadata } from "@pylonsync/react";
import { SellForm } from "../../client/SellForm";

export const metadata: Metadata = {
  title: "Sell an item | Reprise",
  description: "List a pre-owned item and receive buyer offers in real time.",
};

export default function SellPage() {
  return (
    <div className="mx-auto max-w-5xl pb-6 pt-8 sm:pt-10">
      <header className="mb-8">
        <h1 className="font-display text-[44px] leading-none sm:text-[52px]">
          Sell an item
        </h1>
        <p className="mt-3 max-w-xl text-sm leading-6 text-muted-foreground">
          The listing goes live in every open browse page as soon as you post
          it. Offers arrive on your dashboard.
        </p>
      </header>
      <SellForm />
    </div>
  );
}
