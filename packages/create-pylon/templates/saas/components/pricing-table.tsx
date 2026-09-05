"use client";

import React, { useState } from "react";
import { PLANS, TRIAL_DAYS, annualSavingsPercent, formatPrice } from "@/lib/plans";

/**
 * The plan cards with a monthly/annual switch. Used on the landing page's
 * pricing section and on /pricing. Prices come from lib/plans.ts, the same
 * catalog the Billing tab and the server cap read.
 */
export function PricingTable({ signedIn }: { signedIn: boolean }) {
  const [annual, setAnnual] = useState(true);
  const savings = Math.max(...PLANS.map(annualSavingsPercent));
  return (
    <div>
      <div className="flex items-center justify-center gap-3 text-sm">
        <span className={annual ? "text-zinc-500" : "font-medium text-zinc-900"}>Monthly</span>
        <button
          type="button"
          role="switch"
          aria-checked={annual}
          aria-label="Bill annually"
          onClick={() => setAnnual((v) => !v)}
          className={`relative h-6 w-11 rounded-full transition-colors ${annual ? "bg-zinc-900" : "bg-zinc-300"}`}
        >
          <span
            className={`absolute top-0.5 size-5 rounded-full bg-white transition-transform ${annual ? "translate-x-5" : "translate-x-0.5"}`}
          />
        </button>
        <span className={annual ? "font-medium text-zinc-900" : "text-zinc-500"}>
          Annual{savings > 0 ? ` · save ${savings}%` : ""}
        </span>
      </div>

      <div className="mt-10 grid gap-5 md:grid-cols-2">
        {PLANS.map((p) => {
          const featured = p.id === "pro";
          const perMonth = annual && p.annualPerMonth != null ? p.annualPerMonth : p.monthly;
          const href = signedIn
            ? featured
              ? `/dashboard/billing?upgrade=pro&interval=${annual ? "annual" : "monthly"}`
              : "/dashboard"
            : "/signup";
          return (
            <div
              key={p.id}
              className={`flex flex-col rounded-2xl border p-7 ${
                featured
                  ? "border-zinc-900 bg-white shadow-[0_24px_60px_-30px_rgba(0,0,0,0.3)]"
                  : "border-zinc-200 bg-paper"
              }`}
            >
              <div className="flex items-center justify-between">
                <h3 className="text-base font-semibold">{p.name}</h3>
                {featured && (
                  <span className="font-mono text-[10px] uppercase tracking-[0.12em] text-brand">
                    {TRIAL_DAYS}-day free trial
                  </span>
                )}
              </div>
              <p className="mt-1 text-[13px] text-zinc-500">{p.tagline}</p>
              <div className="mt-5 flex items-baseline gap-1">
                <span className="text-4xl font-semibold tracking-tight">{formatPrice(perMonth)}</span>
                <span className="text-[13px] text-zinc-500">
                  {p.monthly === 0 ? "forever" : `/ month${annual ? ", billed yearly" : ""}`}
                </span>
              </div>
              <ul className="mt-6 flex-1 space-y-3 text-[14px] text-zinc-600">
                {p.features.map((f) => (
                  <li key={f} className="flex gap-2.5">
                    <span className="mt-[3px] text-brand">✓</span>
                    {f}
                  </li>
                ))}
              </ul>
              <div className="mt-7">
                <a
                  href={href}
                  className={
                    featured
                      ? "inline-flex h-10 w-full items-center justify-center rounded-lg bg-zinc-900 text-sm font-medium text-white transition-colors hover:bg-zinc-700"
                      : "inline-flex h-10 w-full items-center justify-center rounded-lg border border-zinc-300 bg-white text-sm font-medium text-zinc-900 transition-colors hover:bg-zinc-50"
                  }
                >
                  {signedIn && featured ? "Upgrade to Pro" : p.cta}
                </a>
              </div>
            </div>
          );
        })}
      </div>
      <p className="mt-6 text-center text-xs text-zinc-400">
        Prices in USD. The trial needs a card and converts to the plan you pick unless you cancel first.
      </p>
    </div>
  );
}
