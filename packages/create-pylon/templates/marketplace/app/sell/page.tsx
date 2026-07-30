import React from "react";
import { type Metadata } from "@pylonsync/react";
import { SellForm } from "../../client/SellForm";

export const metadata: Metadata = {
  title: "Sell an item | Reprise",
  description: "Create a polished listing and receive buyer offers in real time.",
};

export default function SellPage() {
  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-8 py-2 sm:py-8">
      <header className="flex flex-col gap-2">
        <h1 className="text-balance text-3xl font-semibold tracking-[-0.03em]">
          List something worth finding
        </h1>
        <p className="max-w-xl text-pretty text-sm leading-6 text-muted-foreground">
          Your listing goes live instantly. Offers arrive in your{" "}
          <a href="/me" className="underline">
            dashboard
          </a>{" "}
          in real time.
        </p>
      </header>
      <SellForm />
    </div>
  );
}
