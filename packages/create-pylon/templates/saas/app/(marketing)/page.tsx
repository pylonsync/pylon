import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { ArrowRight } from "lucide-react";
import {
  WRAP,
  FeatureList,
  PrimaryButton,
  ProductShot,
  SecondaryButton,
} from "@/components/marketing";
import { PricingTable } from "@/components/pricing-table";
import { siteConfig } from "@/lib/site.config";

// SEO metadata, rendered into <head> on the server. All copy lives in
// lib/site.config.ts; edit it there.
export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
};

// `(marketing)/page.tsx` → `/`. The landing page: hero with a product shot,
// one section per product, a customer quote, pricing, FAQ, and a closing
// call to action. It reads `auth` during the server render, so the buttons
// say "Get started" to visitors and "Open dashboard" to signed-in people on
// the first byte. The screenshots are captures of this app's own dashboard
// (public/screenshots, made by scripts/capture-screenshots.mjs).
export default function LandingPage({ auth }: PageProps) {
  const signedIn = Boolean(auth.user_id);
  const primaryHref = signedIn ? "/dashboard" : "/signup";
  const primaryLabel = signedIn ? "Open dashboard" : "Get started";
  const { hero, products, quote, pricing, faq, finalCta } = siteConfig;

  return (
    <div className="overflow-x-clip bg-white text-zinc-900">
      {/* ============================ HERO ============================ */}
      <section className="relative">
        <div
          aria-hidden
          className="pointer-events-none absolute inset-x-0 top-0 h-[720px] bg-[radial-gradient(60%_55%_at_50%_0%,var(--brand-soft),transparent_70%)]"
        />
        <div
          aria-hidden
          className="pointer-events-none absolute inset-x-0 top-0 h-[720px] bg-[linear-gradient(to_right,rgba(24,24,27,0.045)_1px,transparent_1px),linear-gradient(to_bottom,rgba(24,24,27,0.045)_1px,transparent_1px)] bg-[size:56px_56px] [mask-image:radial-gradient(70%_60%_at_50%_0%,black,transparent_75%)]"
        />
        <div className={`${WRAP} relative pb-10 pt-14 sm:pt-20`}>
          <div className="mx-auto max-w-4xl text-center">
            <h1 className="text-balance text-[2.3rem] font-semibold leading-[1.04] tracking-[-0.04em] text-zinc-950 sm:text-[3.6rem]">
              {hero.headline}
            </h1>
            <p className="mx-auto mt-5 max-w-xl text-balance text-[17px] leading-relaxed text-zinc-500 sm:text-[18px]">
              {hero.subcopy}
            </p>
            <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
              <PrimaryButton href={primaryHref}>{primaryLabel}</PrimaryButton>
              <SecondaryButton href="/pricing">See pricing</SecondaryButton>
            </div>
            <p className="mt-4 text-[13px] text-zinc-400">{hero.note}</p>
          </div>
          <div className="relative mx-auto mt-12 max-w-[1120px] sm:mt-14">
            <ProductShot src={hero.screenshot} alt={hero.screenshotAlt} priority />
          </div>
        </div>
      </section>

      {/* ========================== PRODUCTS ========================== */}
      <section id="features" className="scroll-mt-20">
        {products.filter((p) => p.onLanding).map((p, i) => (
          <div key={p.slug} className={`${WRAP} py-16 sm:py-20`}>
            <div
              className={`grid items-center gap-10 lg:gap-14 ${
                i % 2 === 1
                  ? "lg:grid-cols-[minmax(0,1.3fr)_minmax(0,0.9fr)]"
                  : "lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.3fr)]"
              }`}
            >
              <div className={i % 2 === 1 ? "lg:order-2" : ""}>
                <span className="flex size-9 items-center justify-center rounded-xl bg-white text-zinc-700 shadow-[0_0_0_1px_rgba(0,0,0,0.07),0_2px_4px_-1px_rgba(0,0,0,0.06)]">
                  <p.icon className="size-[18px]" strokeWidth={1.75} />
                </span>
                <h2 className="mt-5 text-balance text-[1.9rem] font-semibold leading-[1.1] tracking-[-0.03em] text-zinc-950 sm:text-[2.4rem]">
                  {p.headline}
                </h2>
                <p className="mt-4 max-w-md text-[15.5px] leading-relaxed text-zinc-500">{p.summary}</p>
                <div className="mt-8">
                  <FeatureList items={p.features} />
                </div>
                <Link
                  href={`/products/${p.slug}`}
                  className="group mt-8 inline-flex items-center gap-1.5 text-[14px] font-medium text-zinc-900"
                >
                  More about {p.title.toLowerCase()}
                  <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" />
                </Link>
              </div>
              <ProductShot
                src={p.screenshot}
                alt={p.screenshotAlt}
                crop={p.crop}
                className={i % 2 === 1 ? "lg:order-1" : ""}
              />
            </div>
          </div>
        ))}
      </section>

      {/* ============================ QUOTE =========================== */}
      {quote.quote ? (
        <section className="border-y border-zinc-200/70 bg-zinc-50/70">
          <figure className={`${WRAP} py-20 sm:py-28`}>
            <blockquote className="mx-auto max-w-3xl text-balance text-center text-[1.6rem] font-medium leading-[1.3] tracking-[-0.02em] text-zinc-900 sm:text-[2.1rem]">
              &ldquo;{quote.quote}&rdquo;
            </blockquote>
            <figcaption className="mt-8 text-center text-[14px]">
              <span className="font-semibold text-zinc-900">{quote.name}</span>
              <span className="text-zinc-500"> · {quote.role}</span>
            </figcaption>
          </figure>
        </section>
      ) : null}

      {/* =========================== PRICING ========================== */}
      <section id="pricing" className={`${WRAP} scroll-mt-20 py-20 sm:py-28`}>
        <div className="mx-auto max-w-2xl text-center">
          <h2 className="text-[2rem] font-semibold tracking-[-0.03em] text-zinc-950 sm:text-[2.6rem]">
            {pricing.headline}
          </h2>
          <p className="mt-4 text-[15.5px] leading-relaxed text-zinc-500">{pricing.body}</p>
        </div>
        <div className="mx-auto mt-12 max-w-4xl">
          <PricingTable signedIn={signedIn} />
        </div>
      </section>

      {/* ============================= FAQ ============================ */}
      <section id="faq" className={`${WRAP} scroll-mt-20 pb-20 sm:pb-28`}>
        <div className="grid gap-10 lg:grid-cols-[minmax(0,0.7fr)_minmax(0,1.3fr)]">
          <h2 className="text-[2rem] font-semibold tracking-[-0.03em] text-zinc-950 sm:text-[2.6rem]">{faq.headline}</h2>
          <div className="divide-y divide-zinc-200/80 border-y border-zinc-200/80">
            {faq.items.map((f) => (
              <details key={f.q} className="group py-5">
                <summary className="flex cursor-pointer list-none items-center justify-between gap-6 text-[15.5px] font-medium text-zinc-900 marker:hidden [&::-webkit-details-marker]:hidden">
                  {f.q}
                  <span className="relative size-4 shrink-0 text-zinc-400" aria-hidden>
                    <span className="absolute left-0 top-1/2 h-px w-4 bg-current" />
                    <span className="absolute left-1/2 top-0 h-4 w-px bg-current transition-transform duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] group-open:rotate-90" />
                  </span>
                </summary>
                <p className="mt-3 max-w-2xl text-[14.5px] leading-relaxed text-zinc-500">{f.a}</p>
              </details>
            ))}
          </div>
        </div>
      </section>

      {/* ========================= FINAL CTA ========================== */}
      <section className={`${WRAP} pb-20 sm:pb-28`}>
        <div className="relative overflow-hidden rounded-[28px] bg-zinc-950 px-6 py-16 text-center sm:px-12 sm:py-20">
          <div
            aria-hidden
            className="pointer-events-none absolute inset-0 bg-[radial-gradient(50%_80%_at_50%_0%,color-mix(in_srgb,var(--brand)_40%,transparent),transparent_70%)]"
          />
          <div className="relative">
            <h2 className="text-balance text-[2rem] font-semibold tracking-[-0.03em] text-white sm:text-[2.6rem]">
              {finalCta.headline}
            </h2>
            <p className="mx-auto mt-4 max-w-lg text-[15.5px] leading-relaxed text-zinc-400">{finalCta.body}</p>
            <div className="mt-8 flex justify-center">
              <Link
                href={primaryHref}
                className="group inline-flex h-10 items-center gap-2 rounded-full bg-white pl-5 pr-4 text-[14px] font-medium text-zinc-950 transition-[background-color,transform] duration-200 hover:bg-zinc-100 active:scale-[0.98]"
              >
                {signedIn ? "Open dashboard" : finalCta.cta}
                <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" />
              </Link>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
