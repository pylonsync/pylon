import React from "react";
import { type Metadata } from "@pylonsync/react";
import { WRAP, Divider, ImagePlaceholder, LiveBadge } from "@/components/marketing";
import { BookingWidget } from "./booking-widget";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: {
    title: siteConfig.seo.title,
    description: siteConfig.seo.description,
    type: "website",
  },
};

// `app/(marketing)/page.tsx` → `/` (the group segment adds no URL prefix).
// Server-rendered single-page site for an appointment business, laid out like
// a barbershop poster: cream page, condensed uppercase display type, full-width
// rules between sections. Hero, price list, reviews, location, and FAQ are
// static server HTML (SEO + first paint); the booking section (#book) is a
// client island (<BookingWidget>) with live slot availability. All copy comes
// from siteConfig.
//
// Does not read `auth`, so the public page stays cacheable; the nav's signed-in
// state is resolved in the layout.
export default function LandingPage() {
  const { hero, services, booking, reviews, location, faq } = siteConfig;

  return (
    <div className="bg-cream text-ink">
      {/* ============================= HERO ============================= */}
      <section className={`${WRAP} pt-10 pb-12 sm:pt-14`}>
        <h1 className="font-display text-balance text-[4.25rem] leading-[0.9] tracking-[0.005em] sm:text-[7rem] lg:text-[9.5rem]">
          {hero.headline}
        </h1>

        <div className="mt-8 grid gap-8 lg:mt-10 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)] lg:gap-12">
          <div className="flex flex-col justify-between gap-8">
            <p className="max-w-xl text-[17px] leading-relaxed sm:text-[19px]">{hero.subcopy}</p>
            <div>
              <div className="flex flex-wrap items-center gap-4">
                <a
                  href="#book"
                  className="inline-flex h-12 items-center bg-brand px-7 font-display text-[22px] tracking-[0.04em] text-cream transition-opacity hover:opacity-90"
                >
                  {hero.ctaLabel}
                </a>
                <a
                  href="#services"
                  className="font-display text-[22px] tracking-[0.04em] underline decoration-1 underline-offset-[6px] hover:text-brand"
                >
                  See the prices
                </a>
              </div>
              <dl className="mt-10 grid grid-cols-3 gap-4 border-t border-ink pt-5">
                <QuickFact label="Hours" value={hero.quickFacts.hours} />
                <QuickFact label="Area" value={hero.quickFacts.area} />
                <QuickFact label="Call" value={hero.quickFacts.phone} />
              </dl>
            </div>
          </div>

          {/* Hero photo. Swap public/images/hero.jpg for a shot of your shop. */}
          <div className="relative w-full">
            <ImagePlaceholder shape="portrait" title="The shop" src="/images/hero.jpg" />
            <div className="absolute left-4 top-4">
              <LiveBadge>Availability updates live</LiveBadge>
            </div>
          </div>
        </div>
      </section>

      {/* =========================== SERVICES ========================== */}
      <Divider />
      <section id="services" className={`${WRAP} py-12`}>
        <h2 className="font-display text-[3rem] leading-none sm:text-[4rem]">{services.headline}</h2>
        <ol className="mt-8 border-t border-ink">
          {services.items.map((s) => (
            <li key={s.slug} className="border-b border-ink py-5">
              <div className="flex items-baseline gap-4">
                <h3 className="shrink-0 font-display text-[2rem] leading-none sm:text-[2.5rem]">
                  {s.name}
                </h3>
                <span className="hidden shrink-0 text-[13px] text-ink/60 sm:inline">
                  {s.durationMin} min
                </span>
                <span
                  aria-hidden
                  className="mb-[0.35em] min-w-6 flex-1 border-b border-dotted border-ink/60"
                />
                <span className="shrink-0 font-display text-[2rem] leading-none text-brand sm:text-[2.5rem]">
                  {s.price}
                </span>
              </div>
              {s.description ? (
                <p className="mt-2 max-w-xl text-[14px] leading-relaxed text-ink/70">
                  <span className="sm:hidden">{s.durationMin} min. </span>
                  {s.description}
                </p>
              ) : null}
            </li>
          ))}
        </ol>
      </section>

      {/* ============================ BOOKING ========================== */}
      {booking.enabled ? (
        <section id="book" className="bg-ink text-cream">
          <div className={`${WRAP} py-14`}>
            <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_minmax(0,2fr)] lg:gap-12">
              <div>
                <h2 className="font-display text-[3rem] leading-none sm:text-[4rem]">
                  {booking.headline}
                </h2>
                <p className="mt-5 max-w-sm text-[15px] leading-relaxed text-cream/75">
                  {booking.subcopy}
                </p>
                <p className="mt-6 text-[13px] text-cream/60">
                  Or call{" "}
                  <a href={`tel:${location.phone}`} className="text-cream underline underline-offset-4">
                    {location.phone}
                  </a>
                </p>
              </div>
              <BookingWidget />
            </div>
          </div>
        </section>
      ) : null}

      {/* ============================ REVIEWS ========================== */}
      {reviews && reviews.items.length > 0 ? (
        <section className={`${WRAP} py-12`}>
          <h2 className="font-display text-[3rem] leading-none sm:text-[4rem]">{reviews.headline}</h2>
          <div className="mt-8 grid gap-10 border-t border-ink pt-8 md:grid-cols-3 md:gap-8">
            {reviews.items.map((r) => (
              <figure key={r.name}>
                <blockquote className="font-display text-balance text-[1.75rem] leading-[1.05] sm:text-[2rem]">
                  &ldquo;{r.quote}&rdquo;
                </blockquote>
                <figcaption className="mt-4 text-[13px] text-ink/70">
                  {r.name}
                  {r.rating ? (
                    <span className="ml-2 text-brand" aria-label={`${r.rating} out of 5`}>
                      {r.rating}/5
                    </span>
                  ) : null}
                </figcaption>
              </figure>
            ))}
          </div>
        </section>
      ) : null}

      {/* =========================== LOCATION ========================== */}
      <Divider />
      <section id="visit" className={`${WRAP} py-12`}>
        <div className="grid gap-8 lg:grid-cols-2 lg:gap-12">
          <div>
            <h2 className="font-display text-[3rem] leading-none sm:text-[4rem]">{location.headline}</h2>
            <dl className="mt-6 grid gap-5 text-[15px] leading-relaxed">
              <div>
                <dt className="text-[11px] font-medium uppercase tracking-wide text-ink/60">Address</dt>
                <dd className="mt-1">{location.address}</dd>
              </div>
              <div>
                <dt className="text-[11px] font-medium uppercase tracking-wide text-ink/60">Hours</dt>
                <dd className="mt-1">{location.hoursText}</dd>
              </div>
              <div>
                <dt className="text-[11px] font-medium uppercase tracking-wide text-ink/60">Contact</dt>
                <dd className="mt-1">
                  <a href={`tel:${location.phone}`} className="text-brand">
                    {location.phone}
                  </a>{" "}
                  ·{" "}
                  <a href={`mailto:${location.email}`} className="underline underline-offset-4">
                    {location.email}
                  </a>
                </dd>
              </div>
            </dl>
            <a
              href="#book"
              className="mt-8 inline-flex h-12 items-center bg-ink px-7 font-display text-[22px] tracking-[0.04em] text-cream transition-opacity hover:opacity-90"
            >
              {hero.ctaLabel}
            </a>
          </div>
          {location.mapEmbedUrl ? (
            <iframe
              title="Map"
              src={location.mapEmbedUrl}
              className="h-72 w-full border border-ink"
              loading="lazy"
            />
          ) : (
            <div className="grid h-72 place-items-center border border-ink bg-paper text-sm text-ink/60">
              Drop a Google Maps embed URL in <code className="mx-1">location.mapEmbedUrl</code>
            </div>
          )}
        </div>
      </section>

      {/* ============================== FAQ ============================ */}
      {faq && faq.items.length > 0 ? (
        <>
          <Divider />
          <section className={`${WRAP} py-12`}>
            <h2 className="font-display text-[3rem] leading-none sm:text-[4rem]">{faq.headline}</h2>
            <div className="mt-8 divide-y divide-ink border-y border-ink">
              {faq.items.map((f) => (
                <details key={f.q} className="group py-5">
                  <summary className="flex cursor-pointer items-center justify-between text-[16px] font-medium marker:hidden [&::-webkit-details-marker]:hidden">
                    {f.q}
                    <span className="font-display text-[24px] leading-none text-brand transition-transform group-open:rotate-45">
                      +
                    </span>
                  </summary>
                  <p className="mt-3 max-w-2xl text-[14.5px] leading-relaxed text-ink/70">{f.a}</p>
                </details>
              ))}
            </div>
          </section>
        </>
      ) : null}
    </div>
  );
}

function QuickFact({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-[11px] font-medium uppercase tracking-wide text-ink/60">{label}</dt>
      <dd className="mt-1 text-[14px] font-medium">{value}</dd>
    </div>
  );
}
