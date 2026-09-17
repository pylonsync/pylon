import React, { Suspense, use } from "react";
import { Link, type Metadata, type PageProps, type ServerData } from "@pylonsync/react";
import { WRAP, TEXT_LINK, ProjectCard } from "@/components/marketing";
import { SeedProjects } from "../seeder";
import { siteConfig } from "@/lib/site.config";
import { slugify, viewFromRow, type ProjectRow, type ProjectView } from "@/lib/agency";

export const metadata: Metadata = {
  title: `Work — ${siteConfig.brand.name}`,
  description: `Case studies from ${siteConfig.brand.name}.`,
  openGraph: { title: `Work — ${siteConfig.brand.name}`, type: "website" },
};

// `app/work/page.tsx` → `/work`. The full portfolio: every PUBLISHED project,
// server-rendered for SEO, each linking to its case study. Reads the Project
// entity via serverData; falls back to the config case studies before the
// portfolio is seeded so the page is never empty.
function allFromConfig(): ProjectView[] {
  return siteConfig.work.items.map((c) => ({
    slug: c.slug || slugify(c.title),
    title: c.title,
    client: c.client,
    summary: c.summary,
    year: c.year ?? null,
    tags: c.tags,
  }));
}

const WORK_GRID = "mt-12 grid gap-x-8 gap-y-12 sm:grid-cols-2";

function WorkGrid({ serverData }: { serverData: ServerData }) {
  const rows = use(serverData.list<ProjectRow>("Project"));
  const fromDb = rows
    .filter((p) => p.published)
    .sort((a, b) => a.order - b.order || (a.createdAt < b.createdAt ? -1 : 1))
    .map(viewFromRow);
  const projects = fromDb.length > 0 ? fromDb : allFromConfig();

  if (projects.length === 0) {
    return <p className="mt-12 text-[15px] text-zinc-600">No published work yet.</p>;
  }
  return (
    <div className={WORK_GRID}>
      {projects.map((p) => (
        <ProjectCard key={p.slug} p={p} />
      ))}
    </div>
  );
}

export default function WorkIndexPage({ serverData }: PageProps) {
  return (
    <div className="bg-white text-ink">
      <section className={`${WRAP} pt-14 pb-16 sm:pt-20 sm:pb-20`}>
        <h1 className="font-display text-[2.75rem] leading-[1.02] tracking-[-0.015em] sm:text-[4rem]">
          {siteConfig.work.headline}
        </h1>
        <p className="mt-4 max-w-xl text-[16px] leading-relaxed text-zinc-600">
          Every project we have published, each with a short case study.
        </p>
        <Suspense
          fallback={
            <div className={WORK_GRID}>
              {[0, 1, 2, 3].map((i) => (
                <div key={i}>
                  <div className="aspect-[4/3] animate-pulse rounded-sm bg-zinc-100" />
                  <div className="mt-4 h-5 w-1/3 animate-pulse rounded bg-zinc-100" />
                </div>
              ))}
            </div>
          }
        >
          <WorkGrid serverData={serverData} />
        </Suspense>

        <div className="mt-14 text-[15px]">
          <Link href="/#contact" className={TEXT_LINK}>
            Start a project
          </Link>
        </div>
      </section>

      <SeedProjects />
    </div>
  );
}
