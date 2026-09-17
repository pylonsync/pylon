import React, { Suspense, use } from "react";
import {
  Link,
  type GenerateMetadata,
  type Metadata,
  type PageProps,
  type ServerData,
  type SsrResponse,
} from "@pylonsync/react";
import { WRAP, WRAP_NARROW, TEXT_LINK, ImagePlaceholder } from "@/components/marketing";
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
      <div className={`${WRAP_NARROW} py-24`}>
        <p className="font-display text-[2rem]">That case study does not exist.</p>
        <Link href="/work" className={`mt-4 inline-block text-[15px] ${TEXT_LINK}`}>
          All work
        </Link>
      </div>
    );
  }

  return (
    <article className="pt-10 pb-20 sm:pt-14">
      <header className={WRAP}>
        <Link href="/work" className={`text-[14px] ${TEXT_LINK}`}>
          All work
        </Link>
        <h1 className="font-display mt-8 max-w-4xl text-balance text-[2.75rem] leading-[1.02] tracking-[-0.015em] sm:text-[4rem]">
          {p.title}
        </h1>
        <div className="mt-6 grid gap-6 sm:grid-cols-[1fr_auto] sm:items-end">
          <p className="max-w-xl text-[17px] leading-relaxed text-zinc-700">{p.summary}</p>
          <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1 text-[14px] sm:text-right">
            <dt className="text-zinc-500">Client</dt>
            <dd>{p.client}</dd>
            {p.year ? (
              <>
                <dt className="text-zinc-500">Year</dt>
                <dd>{p.year}</dd>
              </>
            ) : null}
            {p.tags.length > 0 ? (
              <>
                <dt className="text-zinc-500">Scope</dt>
                <dd>{p.tags.join(", ")}</dd>
              </>
            ) : null}
            {p.liveUrl ? (
              <>
                <dt className="text-zinc-500">Site</dt>
                <dd>
                  <a href={p.liveUrl} target="_blank" rel="noopener noreferrer" className={TEXT_LINK}>
                    Visit
                  </a>
                </dd>
              </>
            ) : null}
          </dl>
        </div>

        {/* Product shot: public/images/work/<slug>.jpg. */}
        <div className="mt-10">
          <ImagePlaceholder shape="landscape" title={`${p.title} product shot`} src={`/images/work/${p.slug}.jpg`} />
        </div>
      </header>

      <div className={`${WRAP_NARROW} mt-14 space-y-12`}>
        <CaseSection title="The challenge" body={p.challenge} />
        <CaseSection title="Our approach" body={p.approach} />
        <CaseSection title="The outcome" body={p.outcome} />
        {!p.challenge && !p.approach && !p.outcome ? (
          <p className="text-[17px] leading-relaxed text-zinc-700">
            The full write-up is not published yet. In short, {p.summary.charAt(0).toLowerCase() + p.summary.slice(1)}
          </p>
        ) : null}

        <div className="border-t border-zinc-200 pt-10">
          <h2 className="font-display text-[2rem] leading-tight">Have a project like this?</h2>
          <p className="mt-3 text-[16px] leading-relaxed text-zinc-600">
            We take on a few projects at a time. Tell us what you are building.
          </p>
          <Link href="/#contact" className={`mt-4 inline-block text-[15px] ${TEXT_LINK}`}>
            Start a project
          </Link>
        </div>
      </div>

      <SeedProjects />
    </article>
  );
}

function CaseSection({ title, body }: { title: string; body?: string | null }) {
  if (!body) return null;
  return (
    <section>
      <h2 className="font-display text-[2rem] leading-tight">{title}</h2>
      <p className="mt-4 whitespace-pre-wrap text-[17px] leading-relaxed text-zinc-700">{body}</p>
    </section>
  );
}

// `app/work/[slug]/page.tsx` → `/work/:slug`. The case-study detail. Suspends on
// the project lookup; renders a 404 (with a real status) when the slug is
// unknown or the project is a draft.
export default function CaseStudyPage({ params, serverData, response }: PageProps) {
  return (
    <div className="bg-white text-ink">
      <Suspense
        fallback={
          <div className={`${WRAP} pt-10 sm:pt-14`}>
            <div className="h-4 w-16 animate-pulse rounded bg-zinc-100" />
            <div className="mt-8 h-14 w-2/3 animate-pulse rounded bg-zinc-100" />
            <div className="mt-6 h-4 w-1/2 animate-pulse rounded bg-zinc-100" />
            <div className="mt-10 aspect-[4/3] animate-pulse rounded-sm bg-zinc-100" />
          </div>
        }
      >
        <CaseStudy serverData={serverData} response={response} slug={params.slug} />
      </Suspense>
    </div>
  );
}
