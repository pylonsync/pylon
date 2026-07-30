import React from "react";
import { type Metadata } from "@pylonsync/react";
import { SellForm } from "../../client/SellForm";

export const metadata: Metadata = {
  title: "Sell an item | Reprise",
  description: "Create a polished listing and receive buyer offers in real time.",
};

export default function SellPage() {
  return (
    <div className="mx-auto max-w-2xl space-y-8 py-2 sm:py-8">
      <header className="space-y-2">
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
      <div className="rounded-2xl bg-card p-5 shadow-[var(--shadow-border)] sm:p-7">
        <SellForm />
      </div>
    </div>
  );
}
