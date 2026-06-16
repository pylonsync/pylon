import React from "react";
import { type Metadata } from "@pylonsync/react";
import { WRAP, Eyebrow, Badge, Divider, SectionHead, FeatureGrid, initials } from "@/components/marketing";
import { WaitlistHero } from "./waitlist-hero";
import { siteConfig } from "@/lib/site.config";

// SEO metadata — server-rendered into <head>, so the page is fully indexable
// (view source and the copy is in the HTML). All copy lives in
// lib/site.config.ts; edit it there to rebrand.
export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: {
    title: siteConfig.seo.title,
    description: siteConfig.seo.description,
    type: "website",
  },
};

// `app/page.tsx` → `/`. A server-rendered, single-page coming-soon landing.
// The headline, value props, proof, and FAQ are static server HTML (great for
// SEO + first paint); the email form and live counter are a client island
// (<WaitlistHero>) that hydrates in the browser. Every string is sourced from
// `siteConfig`, so the whole page rebrands from one file.
//
// Note: this page intentionally does NOT read `auth`, so it stays a public,
// cacheable render. Signed-in state (the nav's "Dashboard" link) is resolved in
// the layout.
export default function LandingPage() {
  const { hero, counter, valueProps, socialProof, faq } = siteConfig;

  return (
    <div className="bg-white text-zinc-900">
      {/* ============================= HERO ============================= */}
      <section className={`${WRAP} pt-20 pb-16 sm:pt-28`}>
        <div className="flex flex-col items-center text-center">
          <Badge>{hero.badge}</Badge>
          <h1 className="mt-6 max-w-2xl text-balance text-[2.5rem] font-semibold leading-[1.05] tracking-[-0.02em] sm:text-[3.25rem]">
            {hero.headline}
          </h1>
          <p className="mt-5 max-w-xl text-balance text-[17px] leading-relaxed text-zinc-500">
            {hero.subcopy}
          </p>
        </div>
        <div className="mt-10">
          <WaitlistHero hero={hero} counter={counter} />
        </div>
      </section>

      {/* ========================== VALUE PROPS ========================= */}
      <Divider />
      <section className={`${WRAP} py-16`}>
        <SectionHead eyebrow={valueProps.eyebrow} title={valueProps.headline} />
        <div className="mt-10">
          <FeatureGrid items={valueProps.items} />
        </div>
      </section>

      {/* ========================= SOCIAL PROOF ========================= */}
      {socialProof && socialProof.quotes && socialProof.quotes.length > 0 ? (
        <>
          <Divider />
          <section className={`${WRAP} py-16`}>
            <p className="font-mono text-[11px] uppercase tracking-[0.14em] text-zinc-400">
              {socialProof.label}
            </p>
            <div className="mt-8 grid gap-5 sm:grid-cols-3">
              {socialProof.quotes.map((q) => (
                <figure
                  key={q.name + q.role}
                  className="flex flex-col rounded-2xl border border-zinc-200 bg-paper p-6"
                >
                  <blockquote className="text-[14px] leading-relaxed text-zinc-600">
                    &ldquo;{q.quote}&rdquo;
                  </blockquote>
                  <figcaption className="mt-6 flex items-center gap-3">
                    <span className="flex size-8 items-center justify-center rounded-full bg-zinc-200 text-[11px] font-semibold text-zinc-500">
                      {initials(q.name)}
                    </span>
                    <div className="leading-tight">
                      <div className="text-[13px] font-semibold">{q.name}</div>
                      <div className="text-[12px] text-zinc-500">{q.role}</div>
                    </div>
                  </figcaption>
                </figure>
              ))}
            </div>
          </section>
        </>
      ) : null}

      {/* ============================== FAQ ============================= */}
      {faq && faq.items.length > 0 ? (
        <>
          <Divider />
          <section className={`${WRAP} py-16`}>
            <Eyebrow>{faq.eyebrow}</Eyebrow>
            <h2 className="mt-4 text-balance text-2xl font-semibold leading-[1.15] tracking-[-0.02em] sm:text-3xl">
              {faq.headline}
            </h2>
            <div className="mt-8 divide-y divide-zinc-200/70 border-y border-zinc-200/70">
              {faq.items.map((f) => (
                <details key={f.q} className="group py-5">
                  <summary className="flex cursor-pointer items-center justify-between text-[15px] font-medium text-zinc-900 marker:hidden [&::-webkit-details-marker]:hidden">
                    {f.q}
                    <span className="text-brand transition-transform group-open:rotate-45">+</span>
                  </summary>
                  <p className="mt-3 max-w-2xl text-[14px] leading-relaxed text-zinc-500">
                    {f.a}
                  </p>
                </details>
              ))}
            </div>
          </section>
        </>
      ) : null}
    </div>
  );
}
