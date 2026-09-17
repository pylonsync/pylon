import React, { Suspense, use } from "react";
import { Link, type Metadata, type PageProps, type ServerData } from "@pylonsync/react";
import { WRAP, TEXT_LINK, Divider, SectionTitle, ImagePlaceholder, ProjectCard } from "@/components/marketing";
import { LiveSlots, ContactForm } from "./contact-form";
import { SeedProjects } from "./seeder";
import { siteConfig } from "@/lib/site.config";
import { slugify, viewFromRow, type ProjectRow, type ProjectView } from "@/lib/agency";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// The homepage work grid reads the live Project portfolio on the server (the
// `selected` + `published` ones, ordered), so curating it in the dashboard
// re-curates the homepage. Before the portfolio is seeded, it falls back to the
// config case studies so the section is never empty on first paint.
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

const WORK_GRID = "grid gap-x-8 gap-y-12 sm:grid-cols-2";

function SelectedWork({ serverData }: { serverData: ServerData }) {
  const rows = use(serverData.list<ProjectRow>("Project"));
  const fromDb = rows
    .filter((p) => p.selected && p.published)
    .sort((a, b) => a.order - b.order || (a.createdAt < b.createdAt ? -1 : 1))
    .map(viewFromRow);
  const projects = fromDb.length > 0 ? fromDb : selectedFromConfig();

  return (
    <div className={WORK_GRID}>
      {projects.map((p) => (
        <ProjectCard key={p.slug} p={p} />
      ))}
    </div>
  );
}

function WorkSkeleton({ count }: { count: number }) {
  return (
    <div className={WORK_GRID}>
      {Array.from({ length: count }, (_, i) => (
        <div key={i}>
          <div className="aspect-[4/3] animate-pulse rounded-sm bg-zinc-100" />
          <div className="mt-4 h-5 w-1/3 animate-pulse rounded bg-zinc-100" />
          <div className="mt-2 h-3 w-3/4 animate-pulse rounded bg-zinc-100" />
        </div>
      ))}
    </div>
  );
}

// `app/page.tsx` → `/`. Server-rendered studio site. The hero is one serif
// sentence, then the work grid, services, process, team, testimonials, and
// contact. The live "slots open" line and the contact form (#contact) are
// client islands driven by the public Capacity row. All copy comes from
// siteConfig.
export default function LandingPage({ serverData }: PageProps) {
  const { hero, services, work, process, team, testimonials, contact } = siteConfig;

  return (
    <div className="bg-white text-ink">
      {/* ============================== HERO ============================== */}
      <section className={`${WRAP} pt-14 pb-16 sm:pt-20 sm:pb-20`}>
        <h1 className="font-display max-w-5xl text-balance text-[2.75rem] leading-[1.02] tracking-[-0.015em] sm:text-[4rem] lg:text-[5rem]">
          {hero.headline}
        </h1>
        <div className="mt-8 grid gap-6 sm:mt-10 sm:grid-cols-[1fr_auto] sm:items-end">
          <div className="max-w-xl">
            <p className="text-[17px] leading-relaxed text-zinc-700">{hero.subcopy}</p>
            <div className="mt-5 flex flex-wrap items-center gap-x-6 gap-y-2 text-[15px]">
              <a href="#contact" className={TEXT_LINK}>
                {hero.ctaLabel}
              </a>
              <a href="#work" className={TEXT_LINK}>
                {hero.secondaryCtaLabel}
              </a>
            </div>
          </div>
          <LiveSlots />
        </div>
      </section>

      {/* ============================== WORK ============================= */}
      <section id="work" className={`${WRAP} pb-20`}>
        <Suspense fallback={<WorkSkeleton count={2} />}>
          <SelectedWork serverData={serverData} />
        </Suspense>
        <div className="mt-10 text-[15px]">
          <Link href="/work" className={TEXT_LINK}>
            All work
          </Link>
        </div>
      </section>

      {/* ============================ SERVICES =========================== */}
      <Divider />
      <section id="services" className={`${WRAP} py-16 sm:py-20`}>
        <div className="grid gap-10 lg:grid-cols-[1fr_2fr]">
          <SectionTitle title={services.headline} />
          <ol className="divide-y divide-zinc-200 border-y border-zinc-200">
            {services.items.map((s, i) => (
              <li key={s.title} className="grid gap-2 py-6 sm:grid-cols-[3rem_1fr] sm:gap-6">
                <span className="text-[13px] tabular-nums text-zinc-500">{String(i + 1).padStart(2, "0")}</span>
                <div>
                  <h3 className="font-display text-[1.5rem] leading-tight">{s.title}</h3>
                  <p className="mt-2 max-w-lg text-[15px] leading-relaxed text-zinc-600">{s.body}</p>
                </div>
              </li>
            ))}
          </ol>
        </div>
      </section>

      {/* ============================ PROCESS ============================ */}
      <Divider />
      <section className={`${WRAP} py-16 sm:py-20`}>
        <SectionTitle title={process.headline} />
        <ol className="mt-10 grid gap-8 border-t border-zinc-200 pt-8 sm:grid-cols-2 lg:grid-cols-4">
          {process.steps.map((step, i) => (
            <li key={step.title}>
              <span className="text-[13px] tabular-nums text-zinc-500">{String(i + 1).padStart(2, "0")}</span>
              <h3 className="font-display mt-2 text-[1.5rem] leading-tight">{step.title}</h3>
              <p className="mt-2 text-[15px] leading-relaxed text-zinc-600">{step.body}</p>
            </li>
          ))}
        </ol>
      </section>

      {/* ============================== TEAM ============================= */}
      <Divider />
      <section className={`${WRAP} py-16 sm:py-20`}>
        <SectionTitle title={team.headline} />
        <div className="mt-10 grid grid-cols-2 gap-6 sm:grid-cols-3 sm:gap-8">
          {team.members.map((m) => (
            <div key={m.name}>
              {/* Team headshot: public/images/team/<name>.jpg. */}
              <ImagePlaceholder shape="square" title={m.name} src={`/images/team/${slugify(m.name)}.jpg`} />
              <h3 className="mt-4 text-[15px] font-medium">{m.name}</h3>
              <p className="text-[14px] text-zinc-500">{m.role}</p>
            </div>
          ))}
        </div>
      </section>

      {/* ========================== TESTIMONIALS ========================= */}
      {testimonials ? (
        <>
          <Divider />
          <section className={`${WRAP} py-16 sm:py-20`}>
            <SectionTitle title={testimonials.headline} />
            <div className="mt-10 grid gap-10 border-t border-zinc-200 pt-8 sm:grid-cols-3 sm:gap-8">
              {testimonials.items.map((t) => (
                <figure key={t.name}>
                  <blockquote className="font-display text-[1.375rem] leading-snug">“{t.quote}”</blockquote>
                  <figcaption className="mt-4 text-[14px] leading-tight">
                    <span className="block font-medium">{t.name}</span>
                    <span className="text-zinc-500">{t.role}</span>
                  </figcaption>
                </figure>
              ))}
            </div>
          </section>
        </>
      ) : null}

      {/* ============================= CONTACT =========================== */}
      <section id="contact" className="bg-paper">
        <div className={`${WRAP} py-16 sm:py-20`}>
          <div className="grid gap-10 lg:grid-cols-[1fr_1.2fr] lg:gap-16">
            <div>
              <SectionTitle title={contact.headline} body={contact.subcopy} />
              <div className="mt-6">
                <LiveSlots />
              </div>
            </div>
            <ContactForm />
          </div>
        </div>
      </section>

      {/* Seeds the public portfolio on first visit (idempotent, zero UI). */}
      <SeedProjects />
    </div>
  );
}
