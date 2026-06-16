import React from "react";
import { type Metadata } from "@pylonsync/react";
import { WRAP, Eyebrow, Divider, SectionHead, FeatureGrid, initials } from "@/components/marketing";
import { ShopGrid } from "./shop-client";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// `app/page.tsx` → `/`. Server-rendered storefront. Hero, value props, reviews,
// and policies are static server HTML (SEO + first paint); the product grid
// (#shop) is a client island with live stock. All copy comes from siteConfig;
// the product list seeds the DB on first visit. Doesn't read `auth`, so the
// public page stays cacheable.
export default function LandingPage() {
  const { hero, products, valueProps, reviews, policies } = siteConfig;

  return (
    <div className="bg-white text-zinc-900">
      {/* HERO */}
      <section className={`${WRAP} pt-16 pb-12 sm:pt-20`}>
        <p className="font-mono text-[11px] uppercase tracking-[0.16em] text-brand">{hero.tagline}</p>
        <h1 className="mt-4 max-w-3xl text-balance text-[2.5rem] font-semibold leading-[1.04] tracking-[-0.02em] sm:text-[3.25rem]">
          {hero.headline}
        </h1>
        <p className="mt-5 max-w-xl text-[17px] leading-relaxed text-zinc-500">{hero.subcopy}</p>
        <div className="mt-7">
          <a href="#shop" className="inline-flex items-center rounded-full bg-brand px-5 py-2.5 text-sm font-medium text-white transition-opacity hover:opacity-90">
            {hero.ctaLabel}
          </a>
        </div>
      </section>

      {/* PRODUCTS */}
      <Divider />
      <section id="shop" className={`${WRAP} py-14`}>
        <SectionHead eyebrow={products.eyebrow} title={products.headline} />
        <div className="mt-8">
          <ShopGrid />
        </div>
      </section>

      {/* VALUE PROPS */}
      <Divider />
      <section className={`${WRAP} py-14`}>
        <SectionHead eyebrow={valueProps.eyebrow} title={valueProps.headline} />
        <div className="mt-10">
          <FeatureGrid items={valueProps.items} />
        </div>
      </section>

      {/* REVIEWS */}
      {reviews && reviews.items.length > 0 ? (
        <>
          <Divider />
          <section className={`${WRAP} py-14`}>
            <SectionHead eyebrow={reviews.eyebrow} title={reviews.headline} />
            <div className="mt-8 grid gap-5 sm:grid-cols-3">
              {reviews.items.map((r) => (
                <figure key={r.name} className="flex flex-col rounded-2xl border border-zinc-200 bg-paper p-6">
                  {r.rating ? (
                    <div className="text-[13px] tracking-wide text-brand">
                      {"★".repeat(r.rating)}
                      <span className="text-zinc-200">{"★".repeat(5 - r.rating)}</span>
                    </div>
                  ) : null}
                  <blockquote className="mt-3 text-[14px] leading-relaxed text-zinc-600">&ldquo;{r.quote}&rdquo;</blockquote>
                  <figcaption className="mt-4 flex items-center gap-3">
                    <span className="flex size-8 items-center justify-center rounded-full bg-zinc-200 text-[11px] font-semibold text-zinc-500">
                      {initials(r.name)}
                    </span>
                    <div className="text-[13px] font-semibold">{r.name}</div>
                  </figcaption>
                </figure>
              ))}
            </div>
          </section>
        </>
      ) : null}

      {/* POLICIES */}
      <Divider />
      <section className={`${WRAP} py-14`}>
        <Eyebrow>{policies.eyebrow}</Eyebrow>
        <h2 className="mt-4 text-balance text-2xl font-semibold leading-[1.15] tracking-[-0.02em] sm:text-3xl">{policies.headline}</h2>
        <div className="mt-8">
          <FeatureGrid items={policies.items.map((p) => ({ title: p.title, body: p.body }))} />
        </div>
      </section>
    </div>
  );
}
