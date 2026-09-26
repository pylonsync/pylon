import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { WRAP } from "@/components/marketing";
import { PricingTable } from "@/components/pricing-table";
import { siteConfig } from "@/lib/site.config";
import { TRIAL_DAYS } from "@/lib/plans";

export const metadata: Metadata = {
  title: `Pricing — ${siteConfig.brand.name}`,
  description: `${siteConfig.brand.name} pricing: a free plan, and Pro with a ${TRIAL_DAYS}-day free trial.`,
};

// `/pricing`. Server-rendered so the copy is in the HTML; the monthly/annual
// switch is the one client island. Plan data comes from lib/plans.ts.
export default function PricingPage({ auth }: PageProps) {
  const { pricing, faq } = siteConfig;
  return (
    <div className="bg-white text-zinc-900">
      <section className={`${WRAP} pb-20 pt-16 sm:pt-24`}>
        <div className="mx-auto max-w-2xl text-center">
          <h1 className="text-[2.5rem] font-semibold tracking-[-0.035em] text-zinc-950 sm:text-[3.25rem]">
            {pricing.headline}
          </h1>
          <p className="mt-4 text-[16px] leading-relaxed text-zinc-500">{pricing.body}</p>
        </div>
        <div className="mx-auto mt-12 max-w-4xl">
          <PricingTable signedIn={Boolean(auth.user_id)} />
        </div>
      </section>
      <section className="border-t border-zinc-200/70">
        <div className={`${WRAP} py-16`}>
          <h2 className="text-[1.5rem] font-semibold tracking-[-0.02em]">{faq.headline}</h2>
          <dl className="mt-8 grid gap-x-12 gap-y-8 md:grid-cols-2">
            {faq.items.map((f) => (
              <div key={f.q}>
                <dt className="text-[15px] font-medium text-zinc-900">{f.q}</dt>
                <dd className="mt-1.5 text-[14px] leading-relaxed text-zinc-500">{f.a}</dd>
              </div>
            ))}
          </dl>
        </div>
      </section>
    </div>
  );
}
