import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import {
  WRAP,
  Badge,
  Divider,
  Eyebrow,
  SectionHead,
  FeatureGrid,
  PrimaryButton,
  GhostLink,
  Shot,
  Portrait,
  Terminal,
} from "@/components/marketing";
import { siteConfig, productBySlug, type Product } from "@/lib/site.config";

// SEO metadata. Exported `metadata` is rendered into <head> on the server, so
// this marketing page is fully indexable — view source and the copy is in the
// HTML. All copy lives in lib/site.config.ts; edit it there.
export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
};

// The products the homepage features inline. Each links to its own
// /products/[slug] page for the full story. (Defined once in lib/site.config.ts.)
const projects = productBySlug("projects")!;
const tasks = productBySlug("tasks")!;
const docs = productBySlug("docs")!;
const automations = productBySlug("automations")!;

// `app/page.tsx` → `/`. A server-rendered marketing landing page. It reads
// `auth` (resolved from the session cookie during the render) so the call to
// action is right on the first byte — "Get started" for visitors, "Open
// dashboard" once you're signed in. No client fetch, no flash. Every string is
// sourced from `siteConfig` so the whole page rebrands from one file.
export default function LandingPage({ auth }: PageProps) {
  const signedIn = Boolean(auth.user_id);
  const primaryHref = signedIn ? "/dashboard" : "/signup";
  const primaryLabel = signedIn ? "Open dashboard" : "Get started";

  const {
    hero,
    logoCloud,
    outcomes,
    featuredTestimonial,
    entryPoints,
    engagement,
    customers,
    gettingStarted,
    pricing,
    team,
    finalCta,
    faq,
    brand,
  } = siteConfig;

  return (
    <div className="bg-white text-zinc-900">
      {/* ============================ HERO ============================ */}
      <section className={`${WRAP} pt-20 pb-16 sm:pt-28`}>
        <Badge>{hero.badge}</Badge>
        <h1 className="mt-6 max-w-2xl text-balance text-[2.75rem] font-semibold leading-[1.05] tracking-[-0.02em] sm:text-[3.5rem]">
          {hero.headline}
        </h1>
        <p className="mt-6 max-w-xl text-[17px] leading-relaxed text-zinc-500">
          {hero.subcopy}
        </p>
        <div className="mt-8 flex flex-wrap items-center gap-4">
          <PrimaryButton href={primaryHref}>{primaryLabel}</PrimaryButton>
          <GhostLink href="/#product">Take the tour →</GhostLink>
        </div>

        <div className="mt-16">
          <Shot url={`${brand.domain}/dashboard`} label={hero.mockupLabel} />
        </div>
      </section>

      {/* ========================= LOGO CLOUD ========================= */}
      <section className={`${WRAP} pb-16`}>
        <p className="font-mono text-[11px] uppercase tracking-[0.14em] text-zinc-400">
          {logoCloud.label}
        </p>
        <div className="mt-6 flex flex-wrap items-center gap-x-10 gap-y-4">
          {logoCloud.companies.map((name) => (
            <div
              key={name}
              className="flex items-center gap-2 text-zinc-400"
              title={name}
            >
              <span className="size-5 rounded bg-zinc-200" />
              <span className="text-[13px] font-semibold uppercase tracking-wide">
                {name}
              </span>
            </div>
          ))}
        </div>
      </section>

      {/* ===================== OUTCOMES (2-col) ====================== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <div className="grid gap-12 lg:grid-cols-[0.9fr_1.1fr]">
          <div>
            <Eyebrow>{outcomes.eyebrow}</Eyebrow>
            <h2 className="mt-4 text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
              {outcomes.headline}
            </h2>
            <p className="mt-5 max-w-md text-[15px] leading-relaxed text-zinc-500">
              {outcomes.body}
            </p>
          </div>
          <FeatureGrid columns={2} items={outcomes.items} />
        </div>
      </section>

      {/* ======================= TESTIMONIAL ========================= */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <div className="grid items-center gap-12 lg:grid-cols-[0.8fr_1.2fr]">
          <Portrait name={featuredTestimonial.name} />
          <figure>
            <div className="font-serif text-4xl leading-none text-brand">
              &ldquo;
            </div>
            <blockquote className="mt-4 text-balance text-2xl font-medium leading-[1.35] tracking-[-0.01em] sm:text-[1.75rem]">
              {featuredTestimonial.quote}
            </blockquote>
            <figcaption className="mt-8 border-t border-zinc-200/70 pt-6">
              <div className="text-sm font-semibold">
                {featuredTestimonial.name}
              </div>
              <div className="text-sm text-zinc-500">
                {featuredTestimonial.role}
              </div>
            </figcaption>
          </figure>
        </div>
      </section>

      {/* =================== FEATURE: PROJECTS ======================= */}
      <Divider />
      <ProductSection id="product" product={projects} primaryHref={primaryHref} />

      {/* ===================== ENTRY POINTS ========================= */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow={entryPoints.eyebrow}
          title={entryPoints.title}
          body={entryPoints.body}
        />
        <div className="mt-12 grid gap-5 sm:grid-cols-2">
          {entryPoints.items.map((c, i) => (
            <div
              key={c.title}
              className="rounded-2xl border border-zinc-200 bg-paper p-7"
            >
              <div className="flex items-start justify-between">
                <span className="flex size-9 items-center justify-center rounded-lg bg-brand-soft text-brand">
                  {c.icon}
                </span>
                <span className="font-mono text-[11px] text-zinc-300">
                  0{i + 1}
                </span>
              </div>
              <h3 className="mt-5 text-base font-semibold">{c.title}</h3>
              <p className="mt-2 text-[14px] leading-relaxed text-zinc-500">
                {c.body}
              </p>
            </div>
          ))}
        </div>
      </section>

      {/* ====================== FEATURE: TASKS ===================== */}
      <Divider />
      <ProductSection product={tasks} primaryHref={primaryHref} />

      {/* ====================== FEATURE: DOCS ====================== */}
      <Divider />
      <ProductSection product={docs} primaryHref={primaryHref} />

      {/* ============== ENGAGEMENT (prose + numbered) ============== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <Eyebrow>{engagement.eyebrow}</Eyebrow>
        <h2 className="mt-4 max-w-2xl text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
          {engagement.headline}
        </h2>
        <div className="mt-12 grid gap-12 lg:grid-cols-2">
          <div className="space-y-5 text-[15px] leading-relaxed text-zinc-500">
            {engagement.paragraphs.map((p) => (
              <p key={p}>{p}</p>
            ))}
          </div>
          <ol className="space-y-7">
            {engagement.items.map((e, i) => (
              <li key={e.title} className="flex gap-4">
                <span className="mt-0.5 font-mono text-[11px] text-zinc-300">
                  0{i + 1}
                </span>
                <div>
                  <span className="text-[15px] font-medium text-brand">
                    {e.title}.
                  </span>{" "}
                  <span className="text-[15px] leading-relaxed text-zinc-500">
                    {e.body}
                  </span>
                </div>
              </li>
            ))}
          </ol>
        </div>
      </section>

      {/* ======================= AUTOMATIONS ======================= */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow={automations.eyebrow}
          arrow
          title={automations.headline}
          body={automations.summary}
        />
        <FeatureGrid className="mt-14" items={automations.features} />
        <div className="mt-8">
          <GhostLink href="/products/automations">
            Explore {automations.title} →
          </GhostLink>
        </div>
        <div className="mt-12">
          <Terminal />
        </div>
      </section>

      {/* =================== TESTIMONIAL CARDS ===================== */}
      <Divider />
      <section id="customers" className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow={customers.eyebrow}
          title={customers.title}
          body={customers.body}
        />
        <div className="mt-12 grid gap-5 sm:grid-cols-3">
          {customers.quotes.map((q) => (
            <figure
              key={q.name + q.role}
              className="flex flex-col rounded-2xl border border-zinc-200 bg-paper p-6"
            >
              <blockquote className="text-[14px] leading-relaxed text-zinc-600">
                &ldquo;{q.quote}&rdquo;
              </blockquote>
              <figcaption className="mt-6 flex items-center gap-3">
                <span className="flex size-8 items-center justify-center rounded-full bg-zinc-200 text-[11px] font-semibold text-zinc-500">
                  {initials(q.name)}
                </span>
                <div className="leading-tight">
                  <div className="text-[13px] font-semibold">{q.name}</div>
                  <div className="text-[12px] text-zinc-500">{q.role}</div>
                </div>
              </figcaption>
            </figure>
          ))}
        </div>
      </section>

      {/* ===================== GETTING STARTED ==================== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <Eyebrow>{gettingStarted.eyebrow}</Eyebrow>
        <h2 className="mt-4 max-w-xl text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
          {gettingStarted.headline}
        </h2>
        <p className="mt-5 max-w-md text-[15px] leading-relaxed text-zinc-500">
          {gettingStarted.body}
        </p>
        <div className="mt-8">
          <PrimaryButton href={primaryHref}>{primaryLabel}</PrimaryButton>
        </div>
        <ol className="mt-14 max-w-xl space-y-8">
          {gettingStarted.steps.map((s, i) => (
            <li key={s.title} className="flex gap-5">
              <span className="flex size-7 shrink-0 items-center justify-center rounded-full bg-brand-soft font-mono text-[11px] text-brand">
                0{i + 1}
              </span>
              <div>
                <h3 className="text-[15px] font-semibold">{s.title}</h3>
                <p className="mt-1.5 text-[14px] leading-relaxed text-zinc-500">
                  {s.body}
                </p>
              </div>
            </li>
          ))}
        </ol>
      </section>

      {/* ========================= PRICING ======================== */}
      <Divider />
      <section id="pricing" className={`${WRAP} py-20`}>
        <Eyebrow>{pricing.eyebrow}</Eyebrow>
        <h2 className="mt-4 max-w-xl text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
          {pricing.headline}
        </h2>
        <p className="mt-5 max-w-md text-[15px] leading-relaxed text-zinc-500">
          {pricing.body}
        </p>
        <div className="mt-12 grid gap-5 lg:grid-cols-3">
          {pricing.plans.map((p) => (
            <div
              key={p.name}
              className={`flex flex-col rounded-2xl border p-7 ${
                p.featured
                  ? "border-zinc-900 bg-white shadow-[0_24px_60px_-30px_rgba(0,0,0,0.3)]"
                  : "border-zinc-200 bg-paper"
              }`}
            >
              <div className="flex items-center justify-between">
                <h3 className="text-base font-semibold">{p.name}</h3>
                {p.featured && (
                  <span className="font-mono text-[10px] uppercase tracking-[0.12em] text-brand">
                    Most popular
                  </span>
                )}
              </div>
              <p className="mt-1 text-[13px] text-zinc-500">{p.tagline}</p>
              <div className="mt-5 flex items-baseline gap-1">
                <span className="text-4xl font-semibold tracking-tight">
                  {p.price}
                </span>
                <span className="text-[13px] text-zinc-500">{p.unit}</span>
              </div>
              <ul className="mt-6 flex-1 space-y-3 text-[14px] text-zinc-600">
                {p.features.map((f) => (
                  <li key={f} className="flex gap-2.5">
                    <span className="mt-[3px] text-brand">✓</span>
                    {f}
                  </li>
                ))}
              </ul>
              <div className="mt-7">
                {p.featured ? (
                  <PrimaryButton
                    href={primaryHref}
                    className="w-full justify-center"
                  >
                    {p.cta}
                  </PrimaryButton>
                ) : (
                  <Link
                    href={primaryHref}
                    className="inline-flex w-full items-center justify-center rounded-full border border-zinc-300 px-5 py-2.5 text-sm font-medium text-zinc-900 transition-colors hover:border-zinc-400 hover:bg-zinc-50"
                  >
                    {p.cta}
                  </Link>
                )}
              </div>
            </div>
          ))}
        </div>
      </section>

      {/* ====================== THE TEAM (2-col) ================== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <div className="grid gap-12 lg:grid-cols-[0.9fr_1.1fr]">
          <div>
            <Eyebrow>{team.eyebrow}</Eyebrow>
            <h2 className="mt-4 text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
              {team.headline}
            </h2>
            <p className="mt-5 max-w-md text-[15px] leading-relaxed text-zinc-500">
              {team.body}
            </p>
          </div>
          <FeatureGrid columns={2} items={team.items} />
        </div>
      </section>

      {/* ======================= FINAL CTA ======================== */}
      <Divider />
      <section className={`${WRAP} py-24`}>
        <Eyebrow>{finalCta.eyebrow}</Eyebrow>
        <h2 className="mt-4 max-w-xl text-balance text-[2.5rem] font-semibold leading-[1.05] tracking-[-0.02em] sm:text-[3rem]">
          {finalCta.headline}
        </h2>
        <p className="mt-6 max-w-lg text-[16px] leading-relaxed text-zinc-500">
          {finalCta.bodyLead}
          <span className="rounded bg-brand-soft px-1.5 py-0.5 font-medium text-brand">
            {finalCta.highlight}
          </span>
          {finalCta.bodyTail}
        </p>
        <div className="mt-8">
          <PrimaryButton href={primaryHref}>{finalCta.cta}</PrimaryButton>
        </div>
        <p className="mt-4 text-[12px] text-zinc-400">{finalCta.footnote}</p>
      </section>

      {/* ========================== FAQ =========================== */}
      <Divider />
      <section id="faq" className={`${WRAP} py-20`}>
        <Eyebrow>{faq.eyebrow}</Eyebrow>
        <h2 className="mt-4 text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
          {faq.headline}
        </h2>
        <div className="mt-10 divide-y divide-zinc-200/70 border-y border-zinc-200/70">
          {faq.items.map((f) => (
            <details key={f.q} className="group py-5">
              <summary className="flex cursor-pointer items-center justify-between text-[15px] font-medium text-zinc-900 marker:hidden [&::-webkit-details-marker]:hidden">
                {f.q}
                <span className="text-brand transition-transform group-open:rotate-45">
                  +
                </span>
              </summary>
              <p className="mt-3 max-w-2xl text-[14px] leading-relaxed text-zinc-500">
                {f.a}
              </p>
            </details>
          ))}
        </div>
      </section>
    </div>
  );
}

// A homepage feature section for one product: eyebrow + headline + grid +
// "Explore →" link to its /products/[slug] page + a product mockup.
function ProductSection({
  product,
  primaryHref,
  id,
}: {
  product: Product;
  primaryHref: string;
  id?: string;
}) {
  return (
    <section id={id} className={`${WRAP} py-20`}>
      <SectionHead
        eyebrow={product.eyebrow}
        arrow
        title={product.headline}
        body={product.summary}
      />
      <FeatureGrid className="mt-14" items={product.features.slice(0, 6)} />
      <div className="mt-8">
        <GhostLink href={`/products/${product.slug}`}>
          Explore {product.title} →
        </GhostLink>
      </div>
      <div className="mt-12">
        <Shot url={product.mockupUrl} label={product.mockupLabel} />
      </div>
    </section>
  );
}

// Initials for the testimonial avatars, so the cards look finished without a
// real photo. Drop in an <img> when you have one.
function initials(name: string) {
  return name
    .split(/\s+/)
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
}
