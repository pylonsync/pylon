import React from "react";
import { type Metadata } from "@pylonsync/react";
import {
  WRAP,
  Eyebrow,
  Divider,
  SectionHead,
  ImagePlaceholder,
  initials,
} from "@/components/marketing";
import { LiveSlots, ContactForm } from "./contact-form";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// `app/page.tsx` → `/`. Server-rendered studio site. Hero, services, work,
// process, team, and testimonials are static server HTML (SEO + first paint);
// the live "slots open" pill and the contact form (#contact) are client islands
// driven by the public Capacity row. All copy comes from siteConfig. Doesn't
// read `auth`, so the public page stays cacheable.
export default function LandingPage() {
  const { hero, logos, services, work, process, team, testimonials, contact } = siteConfig;

  return (
    <div className="bg-white text-zinc-900">
      {/* ============================== HERO ============================== */}
      <section className={`${WRAP} pt-16 pb-14 sm:pt-20`}>
        <div className="grid items-center gap-10 lg:grid-cols-2 lg:gap-14">
          <div>
            <p className="font-mono text-[11px] uppercase tracking-[0.16em] text-brand">{hero.tagline}</p>
            <h1 className="mt-4 text-balance text-[2.5rem] font-semibold leading-[1.04] tracking-[-0.02em] sm:text-[3.25rem]">
              {hero.headline}
            </h1>
            <p className="mt-5 max-w-xl text-[17px] leading-relaxed text-zinc-500">{hero.subcopy}</p>
            <div className="mt-7 flex flex-wrap items-center gap-4">
              <a href="#contact" className="inline-flex items-center rounded-full bg-brand px-5 py-2.5 text-sm font-medium text-white transition-opacity hover:opacity-90">
                {hero.ctaLabel}
              </a>
              <a href="#work" className="text-sm font-medium text-zinc-700 hover:text-zinc-900">
                {hero.secondaryCtaLabel} →
              </a>
            </div>
            <div className="mt-8">
              <LiveSlots />
            </div>
          </div>

          {/* Hero photo — replace the placeholder with a real shot (the studio,
              the team at work, a flagship project). */}
          <div className="relative mx-auto w-full max-w-sm lg:max-w-none">
            <ImagePlaceholder
              shape="portrait"
              title="A photo of your studio or work"
              hint="Swap for an <img> in app/page.tsx"
            />
          </div>
        </div>
      </section>

      {/* ============================== LOGOS ============================= */}
      <section className={`${WRAP} pb-6`}>
        <p className="text-center font-mono text-[11px] uppercase tracking-[0.16em] text-zinc-400">
          {logos.eyebrow}
        </p>
        <div className="mt-5 flex flex-wrap items-center justify-center gap-x-10 gap-y-4">
          {logos.names.map((n) => (
            <span key={n} className="text-[15px] font-semibold tracking-tight text-zinc-300">
              {n}
            </span>
          ))}
        </div>
      </section>

      {/* ============================ SERVICES =========================== */}
      <Divider />
      <section id="services" className={`${WRAP} py-16`}>
        <SectionHead eyebrow={services.eyebrow} title={services.headline} />
        <div className="mt-10 grid gap-x-8 gap-y-10 sm:grid-cols-2">
          {services.items.map((s) => (
            <div key={s.title}>
              {s.icon ? (
                <span className="flex size-9 items-center justify-center rounded-lg bg-brand-soft text-brand">
                  {s.icon}
                </span>
              ) : null}
              <h3 className="mt-4 text-[16px] font-semibold text-zinc-900">{s.title}</h3>
              <p className="mt-2 max-w-md text-[14px] leading-relaxed text-zinc-500">{s.body}</p>
            </div>
          ))}
        </div>
      </section>

      {/* ============================== WORK ============================= */}
      <Divider />
      <section id="work" className={`${WRAP} py-16`}>
        <SectionHead eyebrow={work.eyebrow} title={work.headline} />
        <div className="mt-10 grid gap-6 sm:grid-cols-2">
          {work.items.map((c) => (
            <div key={c.title} className="group">
              {/* Case-study image — drop in a real project screenshot. */}
              <ImagePlaceholder
                shape="landscape"
                title={`${c.title} — project shot`}
                hint="Swap for an <img> per case study"
              />
              <div className="mt-4 flex items-baseline justify-between gap-3">
                <h3 className="text-[16px] font-semibold text-zinc-900">{c.title}</h3>
                <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-zinc-400">
                  {c.client}
                </span>
              </div>
              <p className="mt-1.5 text-[14px] leading-relaxed text-zinc-500">{c.summary}</p>
              <div className="mt-3 flex flex-wrap gap-1.5">
                {c.tags.map((t) => (
                  <span key={t} className="rounded-full bg-zinc-100 px-2.5 py-0.5 text-[11px] font-medium text-zinc-600">
                    {t}
                  </span>
                ))}
              </div>
            </div>
          ))}
        </div>
      </section>

      {/* ============================ PROCESS ============================ */}
      <Divider />
      <section className={`${WRAP} py-16`}>
        <SectionHead eyebrow={process.eyebrow} title={process.headline} />
        <ol className="mt-10 grid gap-8 sm:grid-cols-2 lg:grid-cols-4">
          {process.steps.map((step, i) => (
            <li key={step.title}>
              <span className="flex size-8 items-center justify-center rounded-full bg-zinc-900 font-mono text-[12px] font-semibold text-white">
                {i + 1}
              </span>
              <h3 className="mt-4 text-[15px] font-semibold text-zinc-900">{step.title}</h3>
              <p className="mt-2 text-[14px] leading-relaxed text-zinc-500">{step.body}</p>
            </li>
          ))}
        </ol>
      </section>

      {/* ============================== TEAM ============================= */}
      <Divider />
      <section className={`${WRAP} py-16`}>
        <SectionHead eyebrow={team.eyebrow} title={team.headline} />
        <div className="mt-10 grid gap-8 sm:grid-cols-3">
          {team.members.map((m) => (
            <div key={m.name}>
              {/* Team headshot — drop in a real photo. */}
              <ImagePlaceholder shape="square" title="Headshot" hint="Replace per team member" className="max-w-[200px]" />
              <h3 className="mt-4 text-[15px] font-semibold text-zinc-900">{m.name}</h3>
              <p className="text-[13.5px] text-zinc-500">{m.role}</p>
            </div>
          ))}
        </div>
      </section>

      {/* ========================== TESTIMONIALS ========================= */}
      {testimonials ? (
        <>
          <Divider />
          <section className={`${WRAP} py-16`}>
            <SectionHead eyebrow={testimonials.eyebrow} title={testimonials.headline} />
            <div className="mt-10 grid gap-6 sm:grid-cols-3">
              {testimonials.items.map((t) => (
                <figure key={t.name} className="flex flex-col rounded-2xl border border-zinc-200 bg-paper p-6">
                  <blockquote className="flex-1 text-[14px] leading-relaxed text-zinc-700">“{t.quote}”</blockquote>
                  <figcaption className="mt-5 flex items-center gap-3">
                    <span className="flex size-9 items-center justify-center rounded-full bg-zinc-200 text-[11px] font-semibold text-zinc-500">
                      {initials(t.name)}
                    </span>
                    <span className="text-[13px] leading-tight">
                      <span className="block font-medium text-zinc-900">{t.name}</span>
                      <span className="text-zinc-500">{t.role}</span>
                    </span>
                  </figcaption>
                </figure>
              ))}
            </div>
          </section>
        </>
      ) : null}

      {/* ============================= CONTACT =========================== */}
      <Divider />
      <section id="contact" className={`${WRAP} py-16`}>
        <div className="grid gap-10 lg:grid-cols-[0.9fr_1.1fr]">
          <div>
            <Eyebrow>{contact.eyebrow}</Eyebrow>
            <h2 className="mt-4 text-balance text-2xl font-semibold leading-[1.15] tracking-[-0.02em] sm:text-3xl">
              {contact.headline}
            </h2>
            <p className="mt-4 max-w-md text-[15px] leading-relaxed text-zinc-500">{contact.subcopy}</p>
            <div className="mt-6">
              <LiveSlots />
            </div>
          </div>
          <ContactForm />
        </div>
      </section>
    </div>
  );
}
