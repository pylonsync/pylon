import React, { Suspense, use } from "react";
import { Link, type Metadata, type PageProps, type ServerData } from "@pylonsync/react";
import {
  WRAP,
  Eyebrow,
  Divider,
  SectionHead,
  ImagePlaceholder,
  ProjectCard,
  initials,
} from "@/components/marketing";
import { LiveSlots, ContactForm } from "./contact-form";
import { SeedProjects } from "./seeder";
import { siteConfig } from "@/lib/site.config";
import { slugify, viewFromRow, type ProjectRow, type ProjectView } from "@/lib/agency";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// The homepage "Selected work" grid reads the live Project portfolio on the
// server (the `selected` + `published` ones, ordered), so curating it in the
// dashboard re-curates the homepage. Before the portfolio is seeded, it falls
// back to the config case studies so the section is never empty on first paint.
function selectedFromConfig(): ProjectView[] {
  return siteConfig.work.items
    .filter((c) => c.selected)
    .map((c) => ({
      slug: c.slug || slugify(c.title),
      title: c.title,
      client: c.client,
      summary: c.summary,
      year: c.year ?? null,
      tags: c.tags,
    }));
}

function SelectedWork({ serverData }: { serverData: ServerData }) {
  const rows = use(serverData.list<ProjectRow>("Project"));
  const fromDb = rows
    .filter((p) => p.selected && p.published)
    .sort((a, b) => a.order - b.order || (a.createdAt < b.createdAt ? -1 : 1))
    .map(viewFromRow);
  const projects = fromDb.length > 0 ? fromDb : selectedFromConfig();

  return (
    <div className="mt-10 grid gap-6 sm:grid-cols-2">
      {projects.map((p) => (
        <ProjectCard key={p.slug} p={p} />
      ))}
    </div>
  );
}

// `app/page.tsx` → `/`. Server-rendered studio site. Hero, services, work,
// process, team, and testimonials are static server HTML (SEO + first paint);
// the live "slots open" pill and the contact form (#contact) are client islands
// driven by the public Capacity row. The "Selected work" grid reads the Project
// portfolio server-side. All other copy comes from siteConfig.
export default function LandingPage({ serverData }: PageProps) {
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
        <div className="flex items-end justify-between gap-4">
          <SectionHead eyebrow={work.eyebrow} title={work.headline} />
          <Link
            href="/work"
            className="hidden shrink-0 text-[13.5px] font-medium text-zinc-600 transition-colors hover:text-zinc-900 sm:inline-flex"
          >
            All work →
          </Link>
        </div>
        <Suspense
          fallback={
            <div className="mt-10 grid gap-6 sm:grid-cols-2">
              {[0, 1].map((i) => (
                <div key={i}>
                  <div className="aspect-[4/3] animate-pulse rounded-2xl bg-zinc-100" />
                  <div className="mt-4 h-4 w-1/3 animate-pulse rounded bg-zinc-100" />
                  <div className="mt-2 h-3 w-3/4 animate-pulse rounded bg-zinc-100" />
                </div>
              ))}
            </div>
          }
        >
          <SelectedWork serverData={serverData} />
        </Suspense>
        <div className="mt-8 sm:hidden">
          <Link href="/work" className="text-[14px] font-medium text-brand">
            See all work →
          </Link>
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

      {/* Seeds the public portfolio on first visit (idempotent, zero UI). */}
      <SeedProjects />
    </div>
  );
}
