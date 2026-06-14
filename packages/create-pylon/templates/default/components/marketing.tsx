import React from "react";
import { Link } from "@pylonsync/react";
import type { SitePage, Comparison } from "@/lib/site";

// Reusable presentational pieces for the marketing pages (homepage +
// /products/[slug]). All server-rendered — no client JS. Restyle here and every
// marketing page follows.

// Shared container: the whole marketing site is a contained, left-aligned column.
export const WRAP = "mx-auto w-full max-w-5xl px-6";

export function Divider() {
  return (
    <div className={WRAP}>
      <div className="border-t border-zinc-200/70" />
    </div>
  );
}

export function Eyebrow({ children }: { children: React.ReactNode }) {
  return (
    <p className="font-mono text-[11px] font-semibold uppercase tracking-[0.14em] text-brand">
      {children}
    </p>
  );
}

export function Badge({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-2 rounded-full border border-zinc-200 bg-white py-1 pl-1 pr-3 text-[13px] text-zinc-600">
      <span className="rounded-full bg-brand-soft px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wide text-brand">
        New
      </span>
      {children}
    </span>
  );
}

export function SectionHead({
  eyebrow,
  title,
  body,
  arrow,
}: {
  eyebrow: string;
  title: string;
  body: string;
  arrow?: boolean;
}) {
  return (
    <div>
      <Eyebrow>
        {eyebrow}
        {arrow ? " →" : ""}
      </Eyebrow>
      <h2 className="mt-4 max-w-2xl text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
        {title}
      </h2>
      <p className="mt-5 max-w-xl text-[15px] leading-relaxed text-zinc-500">
        {body}
      </p>
    </div>
  );
}

export function FeatureGrid({
  items,
  columns = 3,
  className = "",
}: {
  items: { title: string; body: string }[];
  columns?: 2 | 3;
  className?: string;
}) {
  const cols =
    columns === 2 ? "sm:grid-cols-2" : "sm:grid-cols-2 lg:grid-cols-3";
  return (
    <div className={`grid gap-x-8 gap-y-10 ${cols} ${className}`}>
      {items.map((f) => (
        <div key={f.title}>
          <h3 className="text-[15px] font-medium text-brand">{f.title}</h3>
          <p className="mt-2 text-[14px] leading-relaxed text-zinc-500">
            {f.body}
          </p>
        </div>
      ))}
    </div>
  );
}

export function PrimaryButton({
  href,
  children,
  className = "",
}: {
  href: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <Link
      href={href}
      className={`inline-flex items-center rounded-full bg-zinc-900 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:bg-zinc-700 ${className}`}
    >
      {children}
    </Link>
  );
}

export function GhostLink({
  href,
  children,
}: {
  href: string;
  children: React.ReactNode;
}) {
  return (
    <Link
      href={href}
      className="text-sm font-medium text-zinc-700 transition-colors hover:text-zinc-900"
    >
      {children}
    </Link>
  );
}

// Browser-chrome frame around an image placeholder. Drop a real screenshot in
// place of the dashed box.
export function Shot({ url, label }: { url: string; label: string }) {
  return (
    <div className="overflow-hidden rounded-2xl border border-zinc-200 bg-white shadow-[0_30px_70px_-35px_rgba(0,0,0,0.3)]">
      <div className="flex items-center gap-1.5 border-b border-zinc-100 px-4 py-3">
        <span className="size-2.5 rounded-full bg-zinc-200" />
        <span className="size-2.5 rounded-full bg-zinc-200" />
        <span className="size-2.5 rounded-full bg-zinc-200" />
        <span className="mx-auto rounded-md bg-zinc-100 px-10 py-1 text-[11px] text-zinc-400">
          {url}
        </span>
      </div>
      <div className="grid aspect-[16/9] place-items-center bg-zinc-50">
        <div className="flex flex-col items-center gap-2.5 text-zinc-400">
          <span className="flex size-11 items-center justify-center rounded-xl border-2 border-dashed border-zinc-300 text-lg">
            ▦
          </span>
          <p className="text-sm font-medium text-zinc-500">{label}</p>
          <p className="text-xs text-zinc-400">Replace with a screenshot</p>
        </div>
      </div>
    </div>
  );
}

export function Portrait() {
  return (
    <div className="grid aspect-square w-full place-items-center overflow-hidden rounded-2xl border border-zinc-200 bg-zinc-100">
      <div className="flex flex-col items-center gap-2 text-zinc-400">
        <span className="flex size-12 items-center justify-center rounded-full border-2 border-dashed border-zinc-300 text-xl">
          ◐
        </span>
        <p className="text-xs">Portrait placeholder</p>
      </div>
    </div>
  );
}

export function Terminal() {
  return (
    <div className="overflow-hidden rounded-2xl border border-zinc-800 bg-zinc-950 shadow-[0_30px_70px_-35px_rgba(0,0,0,0.6)]">
      <div className="flex items-center gap-1.5 border-b border-white/10 px-4 py-3">
        <span className="size-2.5 rounded-full bg-white/20" />
        <span className="size-2.5 rounded-full bg-white/20" />
        <span className="size-2.5 rounded-full bg-white/20" />
        <span className="ml-3 font-mono text-[11px] text-white/40">
          acme · automation run
        </span>
      </div>
      <div className="space-y-3 p-6 font-mono text-[12.5px] leading-relaxed text-zinc-300">
        <p className="text-zinc-500">▸ Rule · when a task is moved to Done</p>
        <p className="text-brand">
          step 1{" "}
          <span className="text-zinc-500">→ notify the project channel</span>
        </p>
        <p className="text-brand">
          step 2{" "}
          <span className="text-zinc-500">→ close the linked subtasks</span>
        </p>
        <div className="rounded-lg border-l-2 border-brand/60 bg-white/5 px-4 py-3 text-zinc-200">
          <p className="font-semibold">Run complete</p>
          <p className="mt-1 text-zinc-400">
            2 steps ran in 240ms — 1 notification sent, 3 subtasks closed.
            [placeholder]
          </p>
        </div>
        <p className="text-zinc-500">▸ Trigger · on every status change</p>
        <p className="text-brand">
          status{" "}
          <span className="text-zinc-500">→ active · 128 runs this week</span>
        </p>
      </div>
    </div>
  );
}

// A generic content page (solutions / resources / company). Renders a hero, a
// grid of sections, a CTA, and links to its siblings.
export function ContentPage({
  page,
  siblings,
  basePath,
  ctaHref,
}: {
  page: SitePage;
  siblings: SitePage[];
  basePath: string;
  ctaHref: string;
}) {
  const others = siblings.filter((s) => s.slug !== page.slug);
  return (
    <div className="bg-white text-zinc-900">
      <section className={`${WRAP} pt-16 pb-16 sm:pt-20`}>
        <Link
          href="/"
          className="text-[13px] text-zinc-500 transition-colors hover:text-zinc-900"
        >
          ← Home
        </Link>
        <div className="mt-6">
          <Eyebrow>{page.eyebrow}</Eyebrow>
        </div>
        <h1 className="mt-4 max-w-2xl text-balance text-[2.25rem] font-semibold leading-[1.05] tracking-[-0.02em] sm:text-[3rem]">
          {page.title}
        </h1>
        <p className="mt-6 max-w-xl text-[17px] leading-relaxed text-zinc-500">
          {page.summary}
        </p>
        <div className="mt-8">
          <PrimaryButton href={ctaHref}>Get started</PrimaryButton>
        </div>
      </section>

      <Divider />
      <section className={`${WRAP} py-20`}>
        <FeatureGrid items={page.sections} />
      </section>

      {others.length > 0 && (
        <>
          <Divider />
          <section className={`${WRAP} py-16`}>
            <Eyebrow>{page.eyebrow}</Eyebrow>
            <div className="mt-6 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
              {others.map((s) => (
                <Link
                  key={s.slug}
                  href={`${basePath}/${s.slug}`}
                  className="rounded-xl border border-zinc-200 bg-paper px-4 py-3 text-[14px] font-medium text-zinc-700 transition-colors hover:border-zinc-300 hover:bg-white hover:text-zinc-900"
                >
                  {s.navLabel} →
                </Link>
              ))}
            </div>
          </section>
        </>
      )}
    </div>
  );
}

// A comparison page: hero + a feature-by-feature table + links to the other
// comparisons.
export function ComparePage({
  cmp,
  all,
  ctaHref,
}: {
  cmp: Comparison;
  all: Comparison[];
  ctaHref: string;
}) {
  const others = all.filter((c) => c.slug !== cmp.slug);
  return (
    <div className="bg-white text-zinc-900">
      <section className={`${WRAP} pt-16 pb-16 sm:pt-20`}>
        <Link
          href="/"
          className="text-[13px] text-zinc-500 transition-colors hover:text-zinc-900"
        >
          ← Home
        </Link>
        <div className="mt-6">
          <Eyebrow>Compare</Eyebrow>
        </div>
        <h1 className="mt-4 max-w-2xl text-balance text-[2.25rem] font-semibold leading-[1.05] tracking-[-0.02em] sm:text-[3rem]">
          {cmp.title}
        </h1>
        <p className="mt-6 max-w-xl text-[17px] leading-relaxed text-zinc-500">
          {cmp.summary}
        </p>
        <div className="mt-8">
          <PrimaryButton href={ctaHref}>Get started</PrimaryButton>
        </div>
      </section>

      <Divider />
      <section className={`${WRAP} py-16`}>
        <div className="overflow-hidden rounded-2xl border border-zinc-200">
          <table className="w-full text-[14px]">
            <thead>
              <tr className="border-b border-zinc-200 bg-paper text-left">
                <th className="px-5 py-3 font-medium text-zinc-400"></th>
                <th className="px-5 py-3 font-semibold text-zinc-900">Acme</th>
                <th className="px-5 py-3 font-medium text-zinc-500">
                  {cmp.competitor}
                </th>
              </tr>
            </thead>
            <tbody>
              {cmp.rows.map((r) => (
                <tr
                  key={r.dim}
                  className="border-b border-zinc-100 last:border-0"
                >
                  <td className="px-5 py-3.5 text-zinc-600">{r.dim}</td>
                  <td className="px-5 py-3.5 font-medium text-zinc-900">
                    <span className="mr-2 text-brand">✓</span>
                    {r.acme}
                  </td>
                  <td className="px-5 py-3.5 text-zinc-500">{r.them}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      {others.length > 0 && (
        <>
          <Divider />
          <section className={`${WRAP} py-16`}>
            <Eyebrow>More comparisons</Eyebrow>
            <div className="mt-6 grid gap-3 sm:grid-cols-3">
              {others.map((c) => (
                <Link
                  key={c.slug}
                  href={`/compare/${c.slug}`}
                  className="rounded-xl border border-zinc-200 bg-paper px-4 py-3 text-[14px] font-medium text-zinc-700 transition-colors hover:border-zinc-300 hover:bg-white hover:text-zinc-900"
                >
                  {c.navLabel} →
                </Link>
              ))}
            </div>
          </section>
        </>
      )}
    </div>
  );
}
