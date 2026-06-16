import React, { Suspense, use } from "react";
import { Link, type Metadata, type PageProps, type ServerData } from "@pylonsync/react";
import { WRAP, SectionHead, ProjectCard } from "@/components/marketing";
import { SeedProjects } from "../seeder";
import { siteConfig } from "@/lib/site.config";
import { slugify, viewFromRow, type ProjectRow, type ProjectView } from "@/lib/agency";

export const metadata: Metadata = {
  title: `Work — ${siteConfig.brand.name}`,
  description: `Case studies from ${siteConfig.brand.name}: ${siteConfig.work.headline}`,
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

function WorkGrid({ serverData }: { serverData: ServerData }) {
  const rows = use(serverData.list<ProjectRow>("Project"));
  const fromDb = rows
    .filter((p) => p.published)
    .sort((a, b) => a.order - b.order || (a.createdAt < b.createdAt ? -1 : 1))
    .map(viewFromRow);
  const projects = fromDb.length > 0 ? fromDb : allFromConfig();

  if (projects.length === 0) {
    return <p className="mt-10 text-[15px] text-zinc-500">No published work yet.</p>;
  }
  return (
    <div className="mt-10 grid gap-x-6 gap-y-10 sm:grid-cols-2">
      {projects.map((p) => (
        <ProjectCard key={p.slug} p={p} />
      ))}
    </div>
  );
}

export default function WorkIndexPage({ serverData }: PageProps) {
  return (
    <div className="bg-white text-zinc-900">
      <section className={`${WRAP} py-16`}>
        <SectionHead
          eyebrow="Work"
          title={siteConfig.work.headline}
          body="A fuller look at what we've shipped — each one a short case study."
        />
        <Suspense
          fallback={
            <div className="mt-10 grid gap-x-6 gap-y-10 sm:grid-cols-2">
              {[0, 1, 2, 3].map((i) => (
                <div key={i}>
                  <div className="aspect-[4/3] animate-pulse rounded-2xl bg-zinc-100" />
                  <div className="mt-4 h-4 w-1/3 animate-pulse rounded bg-zinc-100" />
                </div>
              ))}
            </div>
          }
        >
          <WorkGrid serverData={serverData} />
        </Suspense>

        <div className="mt-12">
          <Link href="/#contact" className="text-[14px] font-medium text-brand">
            Have a project in mind? Start one →
          </Link>
        </div>
      </section>

      <SeedProjects />
    </div>
  );
}
