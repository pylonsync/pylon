import React from "react";
import { type Metadata } from "@pylonsync/react";
import { ArrowRight } from "lucide-react";
import { WRAP, BUTTON_DARK, BUTTON_LINE, SectionHeading } from "@/components/marketing";
import { ShelfCount } from "@/components/shelf-count";
import { ShopGrid } from "./shop-client";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: {
    title: siteConfig.seo.title,
    description: siteConfig.seo.description,
    type: "website",
    images: [{ url: siteConfig.hero.image, alt: siteConfig.hero.imageAlt }],
  },
};

// `app/(marketing)/page.tsx` → `/` (the group segment adds no URL prefix).
// Server-rendered storefront: a full-bleed hero, the live product grid, the
// studio story, reviews, and shipping terms. Everything except the grid (#shop)
// and the hero's shelf count is static server HTML (SEO + first paint). All
// copy comes from siteConfig; the product list seeds the DB on first visit.
// Doesn't read `auth`, so the public page stays cacheable.
export default function LandingPage() {
  const { hero, products, studio, reviews, policies } = siteConfig;

  return (
    <>
      {/* HERO. Desktop: the photo fills the first screen and the copy sits on
          its open left side. Mobile: the photo stacks above the copy. */}
      <section className="relative overflow-hidden bg-stone md:h-[calc(100svh-100px)] md:min-h-[600px] md:max-h-[940px]">
        <div className="relative aspect-[4/5] overflow-hidden sm:aspect-[16/10] md:absolute md:inset-0 md:aspect-auto">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            src={hero.image}
            alt={hero.imageAlt}
            fetchPriority="high"
            className="settle size-full object-cover object-[78%_50%] md:object-[center_60%]"
          />
          {/* Softens the left edge so the headline reads on any crop. */}
          <div className="absolute inset-0 hidden bg-gradient-to-r from-[#efe5d6]/70 via-[#efe5d6]/10 to-transparent md:block" />
        </div>

        <div className={`${WRAP} relative flex h-full flex-col justify-center py-12 md:py-0`}>
          <div className="max-w-[560px]">
            <h1 className="rise font-display text-[3.25rem] font-light leading-[0.98] tracking-[-0.03em] text-ink sm:text-[4.5rem] lg:text-[5rem] xl:text-[6rem]">
              {hero.headline}
            </h1>
            <p
              className="rise mt-7 max-w-[34ch] text-[17px] leading-[1.6] text-ink/75 sm:text-[18px]"
              style={{ animationDelay: "120ms" }}
            >
              {hero.subcopy}
            </p>
            <div className="rise mt-9 flex flex-wrap items-center gap-3" style={{ animationDelay: "220ms" }}>
              <a href="#shop" className={`${BUTTON_DARK} group`}>
                {hero.ctaLabel}
                <ArrowRight
                  className="size-4 transition-transform duration-500 ease-out-quint group-hover:translate-x-1"
                  strokeWidth={1.5}
                />
              </a>
              <a href="#studio" className={BUTTON_LINE}>
                {hero.secondaryLabel}
              </a>
            </div>
            <div className="rise mt-10" style={{ animationDelay: "320ms" }}>
              <ShelfCount />
            </div>
          </div>
        </div>
      </section>

      {/* PRODUCTS */}
      <section id="shop" className="scroll-mt-16 py-24 sm:py-32">
        <div className={WRAP}>
          <div className="flex flex-col justify-between gap-5 md:flex-row md:items-end">
            <SectionHeading>{products.headline}</SectionHeading>
            <p className="max-w-[40ch] text-[15.5px] leading-relaxed text-ink-2">{products.subcopy}</p>
          </div>
          <div className="mt-14 sm:mt-16">
            <ShopGrid />
          </div>
        </div>
      </section>

      {/* STUDIO: type only. Headline on the left, story and figures on the right. */}
      <section id="studio" className="scroll-mt-16 bg-espresso text-warm-white">
        <div className={`${WRAP} grid gap-12 py-24 sm:py-32 md:grid-cols-[1.1fr_1fr] md:gap-20`}>
          <h2 className="font-display max-w-[14ch] text-[2.75rem] font-light leading-[1] tracking-[-0.03em] text-balance sm:text-[4rem] lg:text-[4.75rem]">
            {studio.headline}
          </h2>
          <div className="md:pt-3">
            <div className="max-w-[46ch] space-y-4 text-[16px] leading-[1.7] text-warm-white/70">
              {studio.body.map((p) => (
                <p key={p}>{p}</p>
              ))}
            </div>
            <dl className="mt-12 grid grid-cols-3 gap-6 border-t border-warm-white/15 pt-8">
              {studio.facts.map((f) => (
                <div key={f.label}>
                  <dt className="sr-only">{f.label}</dt>
                  <dd>
                    <span className="font-display block text-[2rem] font-light leading-none tracking-[-0.02em] sm:text-[2.5rem]">
                      {f.value}
                    </span>
                    <span className="mt-2 block text-[13px] leading-snug text-warm-white/55">{f.label}</span>
                  </dd>
                </div>
              ))}
            </dl>
          </div>
        </div>
      </section>

      {/* REVIEWS */}
      {reviews && reviews.items.length > 0 ? (
        <section className="py-24 sm:py-32">
          <div className={WRAP}>
            <SectionHeading>{reviews.headline}</SectionHeading>
            <div className="mt-14 grid gap-12 md:grid-cols-3 md:gap-10">
              {reviews.items.map((r) => (
                <figure key={r.name} className="flex flex-col border-t border-ink/15 pt-7">
                  <blockquote className="font-display text-[1.5rem] font-light leading-[1.3] tracking-[-0.01em] text-ink">
                    &ldquo;{r.quote}&rdquo;
                  </blockquote>
                  <figcaption className="mt-6 text-[13.5px] text-ink-2">
                    {r.name}
                    {r.product ? <span className="text-ink-2/70">, on {r.product}</span> : null}
                  </figcaption>
                </figure>
              ))}
            </div>
          </div>
        </section>
      ) : null}

      {/* SHIPPING AND RETURNS */}
      <section id="shipping" className="scroll-mt-16 border-t border-ink/10 bg-stone/50 py-24 sm:py-28">
        <div className={`${WRAP} grid gap-12 md:grid-cols-[1fr_1.6fr]`}>
          <SectionHeading>{policies.headline}</SectionHeading>
          <dl className="grid gap-x-12 gap-y-10 sm:grid-cols-2">
            {policies.items.map((p) => (
              <div key={p.title}>
                <dt className="font-display text-[1.375rem] text-ink">{p.title}</dt>
                <dd className="mt-2 text-[15px] leading-relaxed text-ink-2">{p.body}</dd>
              </div>
            ))}
          </dl>
        </div>
      </section>
    </>
  );
}
