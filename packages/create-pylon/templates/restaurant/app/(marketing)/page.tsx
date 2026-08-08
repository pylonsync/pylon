import React from "react";
import { type Metadata } from "@pylonsync/react";
import {
  WRAP,
  Eyebrow,
  Divider,
  SectionHead,
  ImagePlaceholder,
  LiveBadge,
} from "@/components/marketing";
import { ReservationWidget } from "./reservation-widget";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// `app/(marketing)/page.tsx` → `/` (the group segment adds no URL prefix). Server-rendered single-page restaurant site. Hero, menu,
// reviews, location, and FAQ are static server HTML (SEO + first paint); the
// reservation section (#reserve) is a client island with live table
// availability. All copy comes from siteConfig. Doesn't read `auth`, so the
// public page stays cacheable.
export default function LandingPage() {
  const { hero, menu, reservations, reviews, location, faq } = siteConfig;

  return (
    <div className="bg-white text-zinc-900">
      {/* HERO */}
      <section className={`${WRAP} pt-16 pb-14 sm:pt-20`}>
        <div className="grid items-center gap-10 lg:grid-cols-2 lg:gap-14">
          <div>
            <p className="font-mono text-[11px] uppercase tracking-[0.16em] text-brand">{hero.tagline}</p>
            <h1 className="mt-4 text-balance text-[2.5rem] font-semibold leading-[1.04] tracking-[-0.02em] sm:text-[3.25rem]">
              {hero.headline}
            </h1>
            <p className="mt-5 max-w-xl text-[17px] leading-relaxed text-zinc-500">{hero.subcopy}</p>
            <div className="mt-7 flex flex-wrap items-center gap-4">
              <a href="#reserve" className="inline-flex items-center rounded-full bg-brand px-5 py-2.5 text-sm font-medium text-white transition-opacity hover:opacity-90">
                {hero.ctaLabel}
              </a>
              <a href="#menu" className="text-sm font-medium text-zinc-700 hover:text-zinc-900">See the menu →</a>
            </div>
            <dl className="mt-10 grid max-w-lg grid-cols-3 gap-4 border-t border-zinc-200/70 pt-6">
              <QuickFact label="Hours" value={hero.quickFacts.hours} />
              <QuickFact label="Area" value={hero.quickFacts.area} />
              <QuickFact label="Call" value={hero.quickFacts.phone} />
            </dl>
          </div>

          {/* Hero photo — replace the placeholder with a real shot of your space. */}
          <div className="relative mx-auto w-full max-w-sm lg:max-w-none">
            <ImagePlaceholder
              shape="portrait"
              title="A photo of your dining room"
              hint="Swap for an <img> in app/(marketing)/page.tsx"
            />
            <div className="absolute left-4 top-4">
              <LiveBadge>Tables update live</LiveBadge>
            </div>
          </div>
        </div>
      </section>

      {/* MENU */}
      <Divider />
      <section id="menu" className={`${WRAP} py-14`}>
        <SectionHead eyebrow={menu.eyebrow} title={menu.headline} />
        <div className="mt-8 grid gap-10 sm:grid-cols-2">
          {menu.sections.map((sec) => (
            <div key={sec.name}>
              <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-brand">{sec.name}</h3>
              <ul className="mt-4 space-y-4">
                {sec.items.map((it) => (
                  <li key={it.name} className="flex items-baseline justify-between gap-4">
                    <div>
                      <div className="flex items-center gap-2">
                        <span className="text-[15px] font-medium text-zinc-900">{it.name}</span>
                        {it.tags?.map((t) => (
                          <span key={t} className="rounded bg-zinc-100 px-1.5 py-0.5 text-[10px] font-medium uppercase text-zinc-500">
                            {t}
                          </span>
                        ))}
                      </div>
                      {it.desc ? <p className="mt-0.5 text-[13px] leading-relaxed text-zinc-500">{it.desc}</p> : null}
                    </div>
                    <span className="shrink-0 text-[14px] font-medium text-zinc-600">{it.price}</span>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
      </section>

      {/* RESERVE */}
      {reservations.enabled ? (
        <>
          <Divider />
          <section id="reserve" className={`${WRAP} py-14`}>
            <SectionHead eyebrow={reservations.eyebrow} title={reservations.headline} body={reservations.subcopy} />
            <div className="mt-8">
              <ReservationWidget />
            </div>
          </section>
        </>
      ) : null}

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
                  <figcaption className="mt-4 text-[13px] font-semibold text-zinc-900">{r.name}</figcaption>
                </figure>
              ))}
            </div>
          </section>
        </>
      ) : null}

      {/* LOCATION */}
      <Divider />
      <section className={`${WRAP} py-14`}>
        <div className="grid gap-8 lg:grid-cols-[1fr_1fr]">
          <div>
            <SectionHead eyebrow={location.eyebrow} title={location.headline} />
            <div className="mt-6 space-y-3 text-[14px] leading-relaxed text-zinc-600">
              <p>{location.address}</p>
              <p className="text-zinc-500">{location.hoursText}</p>
              <p>
                <a href={`tel:${location.phone}`} className="font-medium text-brand">{location.phone}</a>{" "}·{" "}
                <a href={`mailto:${location.email}`} className="text-zinc-600 hover:text-zinc-900">{location.email}</a>
              </p>
              <a href="#reserve" className="mt-2 inline-flex items-center rounded-full bg-zinc-900 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:bg-zinc-700">
                {siteConfig.hero.ctaLabel}
              </a>
            </div>
          </div>
          {location.mapEmbedUrl ? (
            <iframe title="Map" src={location.mapEmbedUrl} className="h-64 w-full rounded-2xl border border-zinc-200" loading="lazy" />
          ) : (
            <div className="grid h-64 place-items-center rounded-2xl border border-zinc-200 bg-paper text-sm text-zinc-400">
              Drop a Google Maps embed URL in <code className="mx-1">location.mapEmbedUrl</code>
            </div>
          )}
        </div>
      </section>

      {/* FAQ */}
      {faq && faq.items.length > 0 ? (
        <>
          <Divider />
          <section className={`${WRAP} py-14`}>
            <Eyebrow>{faq.eyebrow}</Eyebrow>
            <h2 className="mt-4 text-balance text-2xl font-semibold leading-[1.15] tracking-[-0.02em] sm:text-3xl">{faq.headline}</h2>
            <div className="mt-8 divide-y divide-zinc-200/70 border-y border-zinc-200/70">
              {faq.items.map((f) => (
                <details key={f.q} className="group py-5">
                  <summary className="flex cursor-pointer items-center justify-between text-[15px] font-medium text-zinc-900 marker:hidden [&::-webkit-details-marker]:hidden">
                    {f.q}
                    <span className="text-brand transition-transform group-open:rotate-45">+</span>
                  </summary>
                  <p className="mt-3 max-w-2xl text-[14px] leading-relaxed text-zinc-500">{f.a}</p>
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
      <dt className="text-[11px] font-medium uppercase tracking-wide text-zinc-400">{label}</dt>
      <dd className="mt-1 text-[13.5px] font-medium text-zinc-900">{value}</dd>
    </div>
  );
}
