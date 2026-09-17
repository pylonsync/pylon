import React from "react";
import { type Metadata } from "@pylonsync/react";
import { LiveBadge } from "@/components/marketing";
import { ReservationWidget } from "./reservation-widget";
import { siteConfig, type DayHours } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

const WRAP = "mx-auto w-full max-w-6xl px-6";
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

// `app/(marketing)/page.tsx` → `/`. A single dark page: the room behind the
// hero, opening times, the menu, the live reservation calendar on a cream
// panel, a review, and where to find us. Every word and every image path
// comes from lib/site.config.ts. The page never reads `auth`, so the public
// render stays cacheable; the reservation widget is the one client island.
export default function LandingPage() {
  const { hero, gallery, menu, reservations, reviews, location, faq } = siteConfig;

  return (
    <div className="bg-[var(--ink)] text-[var(--cream)]">
      {/* HERO: the room, full bleed, the words on top. */}
      <section className="relative isolate min-h-[88vh] overflow-hidden">
        <img src={hero.image} alt="" className="absolute inset-0 -z-20 size-full object-cover" />
        <div className="absolute inset-0 -z-10 bg-gradient-to-t from-[var(--ink)] via-[var(--ink)]/60 to-[var(--ink)]/20" />
        <div className={`${WRAP} flex min-h-[88vh] flex-col justify-end pb-16 pt-32`}>
          <h1 className="font-display mt-5 max-w-3xl text-balance text-[3rem] font-medium leading-[1.02] tracking-[-0.01em] sm:text-[4.5rem]">
            {hero.headline}
          </h1>
          <p className="mt-6 max-w-xl text-[17px] leading-relaxed text-[var(--cream-2)]">{hero.subcopy}</p>
          <div className="mt-8 flex flex-wrap items-center gap-4">
            <a href="#reserve" className="inline-flex items-center rounded-full bg-brand px-6 py-3 text-[14px] font-medium text-white transition-opacity hover:opacity-90">
              {hero.ctaLabel}
            </a>
            <a href="#menu" className="text-[14px] font-medium text-[var(--cream)] underline-offset-4 hover:underline">
              See the menu
            </a>
            <span className="ml-auto hidden sm:block">
              <LiveBadge>Tables update live</LiveBadge>
            </span>
          </div>
        </div>
      </section>

      {/* OPENING TIMES */}
      <section className="border-y border-white/10">
        <div className={`${WRAP} grid gap-8 py-10 sm:grid-cols-[1fr_2fr] sm:items-start`}>
          <h2 className="font-display text-[26px] italic">Opening times</h2>
          <dl className="grid grid-cols-2 gap-x-8 gap-y-3 text-[14px] sm:grid-cols-3">
            {DAYS.map((day, i) => (
              <div key={day} className="flex justify-between gap-3 border-b border-white/10 pb-2">
                <dt className="text-[var(--cream-2)]">{day}</dt>
                <dd className="font-medium tabular-nums">{hoursLabel(reservations.hours[i])}</dd>
              </div>
            ))}
          </dl>
        </div>
      </section>

      {/* GALLERY */}
      {gallery.length > 0 ? (
        <section className={`${WRAP} grid gap-3 py-12 sm:grid-cols-3`}>
          {gallery.map((g) => (
            <figure key={g.src} className="group">
              <div className="aspect-[4/3] overflow-hidden rounded-[6px] bg-[var(--ink-2)]">
                <img src={g.src} alt={g.caption} className="size-full object-cover transition-transform duration-500 group-hover:scale-[1.03]" loading="lazy" />
              </div>
              <figcaption className="mt-2 font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--cream-2)]">{g.caption}</figcaption>
            </figure>
          ))}
        </section>
      ) : null}

      {/* MENU */}
      <section id="menu" className={`${WRAP} py-16`}>
        <div className="max-w-2xl">
          <h2 className="font-display text-balance text-[2.25rem] leading-[1.08] sm:text-[3rem]">{menu.headline}</h2>
        </div>
        <div className="mt-12 grid gap-12 md:grid-cols-3">
          {menu.sections.map((sec) => (
            <div key={sec.name}>
              <h3 className="font-display border-b border-white/15 pb-3 text-[22px] italic">{sec.name}</h3>
              <ul className="mt-5 space-y-5">
                {sec.items.map((it) => (
                  <li key={it.name}>
                    <div className="flex items-baseline gap-2">
                      <span className="text-[15px] font-medium">{it.name}</span>
                      <span className="mb-1 flex-1 border-b border-dotted border-white/25" aria-hidden />
                      <span className="text-[14px] tabular-nums text-[var(--cream-2)]">{it.price}</span>
                    </div>
                    {it.desc || it.tags?.length ? (
                      <p className="mt-1 text-[13px] leading-relaxed text-[var(--cream-2)]">
                        {it.desc}
                        {it.tags?.map((t) => (
                          <span key={t} className="ml-2 font-mono text-[10px] uppercase tracking-wide text-brand">
                            {t}
                          </span>
                        ))}
                      </p>
                    ) : null}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
      </section>

      {/* RESERVE: the live calendar on a cream panel. */}
      {reservations.enabled ? (
        <section id="reserve" className="bg-[var(--paper)] text-zinc-900">
          <div className={`${WRAP} py-16`}>
            <div className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
              <div className="max-w-xl">
                <h2 className="font-display text-balance text-[2.25rem] leading-[1.08] sm:text-[3rem]">{reservations.headline}</h2>
                <p className="mt-4 text-[15px] leading-relaxed text-zinc-600">{reservations.subcopy}</p>
              </div>
              <p className="text-[13px] text-zinc-500">
                Parties over {reservations.maxPartySize}? Call{" "}
                <a href={`tel:${location.phone}`} className="font-medium text-zinc-900">
                  {location.phone}
                </a>
              </p>
            </div>
            <div className="mt-8">
              <ReservationWidget />
            </div>
          </div>
        </section>
      ) : null}

      {/* REVIEWS: one voice at a time. */}
      {reviews && reviews.items.length > 0 ? (
        <section className={`${WRAP} py-16`}>
          <h2 className="font-display text-[2rem] leading-[1.1]">{reviews.headline}</h2>
          <div className="mt-6 divide-y divide-white/10">
            {reviews.items.map((r) => (
              <figure key={r.name} className="grid gap-4 py-8 sm:grid-cols-[1fr_auto] sm:items-end">
                <blockquote className="font-display max-w-3xl text-balance text-[1.5rem] leading-[1.3] sm:text-[1.9rem]">
                  “{r.quote}”
                </blockquote>
                <figcaption className="text-[13px] text-[var(--cream-2)]">
                  {r.name}
                  {r.rating ? <span className="ml-2 text-brand">{"★".repeat(r.rating)}</span> : null}
                </figcaption>
              </figure>
            ))}
          </div>
        </section>
      ) : null}

      {/* VISIT */}
      <section id="visit" className="border-t border-white/10">
        <div className={`${WRAP} grid gap-10 py-16 lg:grid-cols-2`}>
          <div>
            <h2 className="font-display text-balance text-[2.25rem] leading-[1.08] sm:text-[3rem]">{location.headline}</h2>
            <div className="mt-6 space-y-2 text-[15px] leading-relaxed text-[var(--cream-2)]">
              <p className="text-[var(--cream)]">{location.address}</p>
              <p>{location.hoursText}</p>
              <p>
                <a href={`tel:${location.phone}`} className="text-[var(--cream)] underline-offset-4 hover:underline">
                  {location.phone}
                </a>{" "}
                · <a href={`mailto:${location.email}`} className="underline-offset-4 hover:underline">{location.email}</a>
              </p>
            </div>
            <a href="#reserve" className="mt-8 inline-flex items-center rounded-full border border-white/25 px-5 py-2.5 text-[14px] font-medium transition-colors hover:bg-white/10">
              {hero.ctaLabel}
            </a>
          </div>
          {location.mapEmbedUrl ? (
            <iframe title="Map" src={location.mapEmbedUrl} className="h-72 w-full rounded-[6px] border border-white/10 grayscale" loading="lazy" />
          ) : (
            <div className="grid h-72 place-items-center rounded-[6px] border border-white/10 bg-[var(--ink-2)] text-[13px] text-[var(--cream-2)]">
              Set <code className="mx-1 font-mono">location.mapEmbedUrl</code> to show a map here
            </div>
          )}
        </div>
      </section>

      {/* FAQ */}
      {faq && faq.items.length > 0 ? (
        <section className="border-t border-white/10">
          <div className={`${WRAP} py-16`}>
            <h2 className="font-display text-[2rem] leading-[1.1]">{faq.headline}</h2>
            <div className="mt-8 max-w-3xl divide-y divide-white/10 border-y border-white/10">
              {faq.items.map((f) => (
                <details key={f.q} className="group py-5">
                  <summary className="flex cursor-pointer items-center justify-between text-[16px] font-medium marker:hidden [&::-webkit-details-marker]:hidden">
                    {f.q}
                    <span className="text-brand transition-transform group-open:rotate-45">+</span>
                  </summary>
                  <p className="mt-3 max-w-2xl text-[14.5px] leading-relaxed text-[var(--cream-2)]">{f.a}</p>
                </details>
              ))}
            </div>
          </div>
        </section>
      ) : null}
    </div>
  );
}

function hoursLabel(h: DayHours): string {
  if (!h) return "Closed";
  return `${clock(h.open)} – ${clock(h.close)}`;
}

function clock(t: string): string {
  const [hh, mm] = t.split(":").map(Number);
  const h12 = ((hh + 11) % 12) + 1;
  return `${h12}${mm ? `:${String(mm).padStart(2, "0")}` : ""}${hh < 12 ? "am" : "pm"}`;
}
