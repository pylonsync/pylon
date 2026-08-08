import React, { Suspense, use } from "react";
import {
  Link,
  type GenerateMetadata,
  type Metadata,
  type PageProps,
  type ServerData,
  type SsrResponse,
} from "@pylonsync/react";
import { ImagePlaceholder } from "@/components/marketing";
import { SeedProjects } from "../../seeder";
import { siteConfig } from "@/lib/site.config";
import { slugify, viewFromRow, type ProjectRow, type ProjectView } from "@/lib/agency";

// Resolve a case study from the URL slug. Prefer the live Project row; if the
// row exists but is a draft (`published === false`) it's NOT public → treat as
// missing. Only when there's NO row at all (the portfolio hasn't been seeded
// yet) do we fall back to the matching config case study, so deep links work on
// a fresh install.
async function resolveProject(
  serverData: ServerData,
  slug: string,
): Promise<ProjectView | null> {
  const row = await serverData.lookup<ProjectRow>("Project", "slug", slug);
  if (row) return row.published ? viewFromRow(row) : null;

  const cfg = siteConfig.work.items.find((c) => (c.slug || slugify(c.title)) === slug);
  if (!cfg) return null;
  return {
    slug: cfg.slug || slugify(cfg.title),
    title: cfg.title,
    client: cfg.client,
    summary: cfg.summary,
    year: cfg.year ?? null,
    tags: cfg.tags,
    challenge: cfg.challenge ?? null,
    approach: cfg.approach ?? null,
    outcome: cfg.outcome ?? null,
    liveUrl: cfg.liveUrl ?? null,
  };
}

export const generateMetadata: GenerateMetadata = async ({
  params,
  serverData,
}): Promise<Metadata> => {
  const p = await resolveProject(serverData, params.slug);
  if (!p) return { title: `Case study not found — ${siteConfig.brand.name}`, robots: "noindex" };
  return {
    title: `${p.title} — ${siteConfig.brand.name}`,
    description: p.summary,
    openGraph: { title: `${p.title} — ${siteConfig.brand.name}`, description: p.summary, type: "article" },
  };
};

// Public case study, no auth read → an ISR candidate. `revalidate` serves it
// from cache with stale-while-revalidate and makes <Link>'s prefetch reusable,
// so a click off the work grid hits cache instead of a live render.
export const revalidate = 300;

const WRAP_NARROW = "mx-auto w-full max-w-3xl px-6";

function CaseStudy({
  serverData,
  response,
  slug,
}: {
  serverData: ServerData;
  response: SsrResponse;
  slug: string;
}) {
  const p = use(resolveProject(serverData, slug));

  if (!p) {
    response.setStatus(404);
    return (
      <div className={`${WRAP_NARROW} py-24 text-center`}>
        <p className="text-[15px] font-medium text-zinc-900">That case study doesn&apos;t exist.</p>
        <Link href="/work" className="mt-2 inline-block text-[14px] font-medium text-brand">
          ← Back to all work
        </Link>
      </div>
    );
  }

  return (
    <article className={`${WRAP_NARROW} py-14`}>
      <Link href="/work" className="text-[13.5px] font-medium text-zinc-500 transition-colors hover:text-zinc-900">
        ← All work
      </Link>

      <header className="mt-6">
        <p className="font-mono text-[11px] uppercase tracking-[0.16em] text-brand">
          {p.client}
          {p.year ? ` · ${p.year}` : ""}
        </p>
        <h1 className="mt-3 text-balance text-[2rem] font-semibold leading-[1.08] tracking-[-0.02em] sm:text-[2.5rem]">
          {p.title}
        </h1>
        <p className="mt-4 max-w-2xl text-[17px] leading-relaxed text-zinc-500">{p.summary}</p>
        {p.tags.length > 0 ? (
          <div className="mt-5 flex flex-wrap gap-1.5">
            {p.tags.map((t) => (
              <span key={t} className="rounded-full bg-zinc-100 px-2.5 py-0.5 text-[11px] font-medium text-zinc-600">
                {t}
              </span>
            ))}
          </div>
        ) : null}
        {p.liveUrl ? (
          <a
            href={p.liveUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="mt-6 inline-flex items-center rounded-full border border-zinc-300 px-4 py-2 text-[13.5px] font-medium text-zinc-700 transition-colors hover:border-zinc-400 hover:text-zinc-900"
          >
            Visit live site ↗
          </a>
        ) : null}
      </header>

      {/* Hero shot — drop in a real project image. */}
      <div className="mt-10">
        <ImagePlaceholder shape="landscape" title={`${p.title} — hero shot`} hint="Swap for an <img> in app/work/[slug]/page.tsx" />
      </div>

      <div className="mt-12 space-y-10">
        <CaseSection label="The challenge" body={p.challenge} />
        <CaseSection label="Our approach" body={p.approach} />
        <CaseSection label="The outcome" body={p.outcome} />
        {!p.challenge && !p.approach && !p.outcome ? (
          <p className="text-[15px] leading-relaxed text-zinc-500">
            A full write-up is on the way. In the meantime, {p.summary.toLowerCase()}
          </p>
        ) : null}
      </div>

      {/* Contact CTA */}
      <div className="mt-14 rounded-2xl border border-zinc-200 bg-paper p-8 text-center">
        <h2 className="text-[18px] font-semibold tracking-tight text-zinc-900">Have something like this in mind?</h2>
        <p className="mx-auto mt-2 max-w-md text-[14px] leading-relaxed text-zinc-500">
          We take on a few projects at a time. Tell us what you&apos;re building.
        </p>
        <Link
          href="/#contact"
          className="mt-5 inline-flex items-center rounded-full bg-brand px-5 py-2.5 text-[14px] font-medium text-white transition-opacity hover:opacity-90"
        >
          Start a project
        </Link>
      </div>

      <SeedProjects />
    </article>
  );
}

function CaseSection({ label, body }: { label: string; body?: string | null }) {
  if (!body) return null;
  return (
    <section>
      <h2 className="font-mono text-[11px] uppercase tracking-[0.16em] text-zinc-400">{label}</h2>
      <p className="mt-3 whitespace-pre-wrap text-[16px] leading-relaxed text-zinc-700">{body}</p>
    </section>
  );
}

// `app/work/[slug]/page.tsx` → `/work/:slug`. The case-study detail. Suspends on
// the project lookup; renders a 404 (with a real status) when the slug is
// unknown or the project is a draft.
export default function CaseStudyPage({ params, serverData, response }: PageProps) {
  return (
    <div className="bg-white text-zinc-900">
      <Suspense
        fallback={
          <div className={`${WRAP_NARROW} py-14`}>
            <div className="h-4 w-20 animate-pulse rounded bg-zinc-100" />
            <div className="mt-6 h-10 w-2/3 animate-pulse rounded bg-zinc-100" />
            <div className="mt-4 h-4 w-full animate-pulse rounded bg-zinc-100" />
            <div className="mt-10 aspect-[4/3] animate-pulse rounded-2xl bg-zinc-100" />
          </div>
        }
      >
        <CaseStudy serverData={serverData} response={response} slug={params.slug} />
      </Suspense>
    </div>
  );
}
