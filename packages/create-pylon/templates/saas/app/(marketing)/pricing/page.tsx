import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { WRAP, Eyebrow, Divider } from "@/components/marketing";
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
      <section className={`${WRAP} pt-20 pb-16`}>
        <Eyebrow>{pricing.eyebrow}</Eyebrow>
        <h1 className="mt-4 max-w-xl text-balance text-4xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-5xl">
          {pricing.headline}
        </h1>
        <p className="mt-5 max-w-md text-[15px] leading-relaxed text-zinc-500">{pricing.body}</p>
        <div className="mt-12">
          <PricingTable signedIn={Boolean(auth.user_id)} />
        </div>
      </section>
      <Divider />
      <section className={`${WRAP} py-16`}>
        <h2 className="text-2xl font-semibold tracking-tight">Questions</h2>
        <dl className="mt-8 grid gap-8 md:grid-cols-2">
          {faq.items.map((f) => (
            <div key={f.q}>
              <dt className="text-sm font-semibold text-zinc-900">{f.q}</dt>
              <dd className="mt-1.5 text-sm leading-relaxed text-zinc-500">{f.a}</dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}
