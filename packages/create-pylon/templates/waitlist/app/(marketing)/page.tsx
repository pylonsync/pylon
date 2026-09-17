import React from "react";
import { type Metadata } from "@pylonsync/react";
import { WRAP, FactList, ProductMock } from "@/components/marketing";
import { SignupForm, LiveCount } from "./waitlist-hero";
import { siteConfig } from "@/lib/site.config";

// SEO metadata: server-rendered into <head>, so the page is fully indexable.
// All copy lives in lib/site.config.ts; edit it there to rebrand.
export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: {
    title: siteConfig.seo.title,
    description: siteConfig.seo.description,
    type: "website",
  },
};

// `app/(marketing)/page.tsx` → `/`. A server-rendered, single-page launch
// page. The headline, product mock, facts, and FAQ are static server HTML; the
// email form and the live count are client islands (<SignupForm>, <LiveCount>)
// that hydrate in the browser. Every string comes from `siteConfig`.
//
// This page does not read `auth`, so it stays a public, cacheable render.
// Signed-in state (the nav's "Dashboard" link) is resolved in the layout.
export default function LandingPage() {
  const { hero, counter, mock, facts, faq } = siteConfig;

  return (
    <div>
      {/* ============================= HERO ============================= */}
      <section className={`${WRAP} grid items-center gap-12 pt-16 pb-20 lg:grid-cols-[minmax(0,5fr)_minmax(0,6fr)] lg:gap-16 lg:pt-24 lg:pb-28`}>
        <div>
          <h1 className="font-display text-balance text-[2.5rem] font-bold leading-[1.02] tracking-[-0.02em] text-chalk sm:text-[3.5rem]">
            {hero.headline}
          </h1>
          <p className="mt-6 max-w-md text-[17px] leading-relaxed text-chalk-2">{hero.subcopy}</p>
          <div className="mt-8 max-w-md">
            <SignupForm hero={hero} />
          </div>
          <p className="font-mono-ui mt-6 text-[13px] text-brand">{hero.launchNote}</p>
        </div>
        <ProductMock
          windowTitle={mock.windowTitle}
          sidebar={mock.sidebar}
          activeSidebarIndex={mock.activeSidebarIndex}
          rows={mock.rows}
        />
      </section>

      {/* ============================= FACTS ============================= */}
      <section className={`${WRAP} pb-20`}>
        <h2 className="font-display text-[1.5rem] font-medium tracking-[-0.01em] text-chalk">
          {facts.headline}
        </h2>
        <div className="mt-8">
          <FactList items={facts.items} />
        </div>
        {counter.enabled ? (
          <div className="mt-12 border-t border-line pt-5">
            <LiveCount seed={counter.seedCount ?? 0} label={counter.label} />
          </div>
        ) : null}
      </section>

      {/* ============================== FAQ ============================= */}
      {faq && faq.items.length > 0 ? (
        <section className={`${WRAP} pb-24`}>
          <h2 className="font-display text-[1.5rem] font-medium tracking-[-0.01em] text-chalk">
            {faq.headline}
          </h2>
          <dl className="mt-8 grid gap-x-8 gap-y-8 sm:grid-cols-2">
            {faq.items.map((f) => (
              <div key={f.q} className="border-t border-line pt-5">
                <dt className="text-[15px] font-medium text-chalk">{f.q}</dt>
                <dd className="mt-2 text-[14px] leading-relaxed text-chalk-2">{f.a}</dd>
              </div>
            ))}
          </dl>
        </section>
      ) : null}
    </div>
  );
}
