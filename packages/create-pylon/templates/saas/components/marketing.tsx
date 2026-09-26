import React from "react";
import { Link } from "@pylonsync/react";
import { ArrowRight, Check } from "lucide-react";
import type { SitePage, Comparison } from "@/lib/site";
import { siteConfig } from "@/lib/site.config";

// Presentational pieces for the marketing pages. All server-rendered, no
// client JS. Restyle here and every marketing page follows.

// Shared container for the marketing site.
export const WRAP = "mx-auto w-full max-w-6xl px-4 sm:px-6";

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
      className={`group inline-flex h-10 items-center gap-2 rounded-full bg-zinc-950 pl-5 pr-4 text-[14px] font-medium text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.12),0_1px_2px_rgba(0,0,0,0.2)] transition-[background-color,transform] duration-200 ease-[cubic-bezier(0.32,0.72,0,1)] hover:bg-zinc-800 active:scale-[0.98] ${className}`}
    >
      {children}
      <ArrowRight className="size-4 transition-transform duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] group-hover:translate-x-0.5" />
    </Link>
  );
}

export function SecondaryButton({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <Link
      href={href}
      className="inline-flex h-10 items-center rounded-full px-4 text-[14px] font-medium text-zinc-700 ring-1 ring-zinc-200 transition-colors hover:bg-zinc-50 hover:text-zinc-950"
    >
      {children}
    </Link>
  );
}

/** A region of a 1440x900 screenshot, as fractions of its width and height. */
export type ShotCrop = {
  left: number;
  top: number;
  width: number;
  /** Width / height of the visible region. */
  ratio: number;
};

/**
 * A product screenshot in a two-layer frame: a tinted outer tray and the
 * image inside it with its own hairline. `crop` shows one region of the
 * full-screen capture, scaled to fill the frame.
 */
export function ProductShot({
  src,
  alt,
  crop,
  priority = false,
  className = "",
}: {
  src: string;
  alt: string;
  crop?: ShotCrop;
  priority?: boolean;
  className?: string;
}) {
  // Screenshots are 1440x900 CSS pixels (captured at 2x).
  const IMAGE_RATIO = 1440 / 900;
  return (
    <div
      className={`rounded-[18px] bg-zinc-950/[0.035] p-1.5 ring-1 ring-zinc-950/[0.06] sm:rounded-[22px] sm:p-2 ${className}`}
    >
      <div
        className="overflow-hidden rounded-[13px] bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.06),0_24px_48px_-24px_rgba(24,24,60,0.25)] sm:rounded-[15px]"
        style={{ aspectRatio: crop ? crop.ratio : IMAGE_RATIO }}
      >
        <img
          src={src}
          alt={alt}
          loading={priority ? "eager" : "lazy"}
          decoding="async"
          width={1440}
          height={900}
          className="block h-auto max-w-none"
          style={
            crop
              ? {
                  width: `${100 / crop.width}%`,
                  // Percentage margins resolve against the frame's WIDTH, so
                  // the top offset is converted from image-height units.
                  marginLeft: `-${(crop.left / crop.width) * 100}%`,
                  marginTop: `-${(crop.top / crop.width / IMAGE_RATIO) * 100}%`,
                }
              : { width: "100%" }
          }
        />
      </div>
    </div>
  );
}

export function FeatureList({ items }: { items: { title: string; body: string }[] }) {
  return (
    <ul className="grid gap-x-8 gap-y-5 sm:grid-cols-2">
      {items.map((f) => (
        <li key={f.title} className="flex gap-3">
          <span className="mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full bg-brand-soft text-brand">
            <Check className="size-3" strokeWidth={2.5} />
          </span>
          <span>
            <span className="block text-[14px] font-medium text-zinc-900">{f.title}</span>
            <span className="mt-0.5 block text-[13.5px] leading-relaxed text-zinc-500">{f.body}</span>
          </span>
        </li>
      ))}
    </ul>
  );
}

function PageHero({ title, summary, ctaHref }: { title: string; summary: string; ctaHref: string }) {
  return (
    <section className={`${WRAP} pb-16 pt-16 sm:pt-24`}>
      <h1 className="max-w-3xl text-balance text-[2.5rem] font-semibold leading-[1.05] tracking-[-0.035em] text-zinc-950 sm:text-[3.25rem]">
        {title}
      </h1>
      <p className="mt-5 max-w-xl text-[17px] leading-relaxed text-zinc-500">{summary}</p>
      <div className="mt-8">
        <PrimaryButton href={ctaHref}>Get started</PrimaryButton>
      </div>
    </section>
  );
}

// A generic content page (solutions / resources / company): heading, a grid
// of sections, and links to its siblings.
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
      <PageHero title={page.title} summary={page.summary} ctaHref={ctaHref} />
      <section className={`${WRAP} pb-20`}>
        <div className="grid gap-px overflow-hidden rounded-2xl bg-zinc-200/70 ring-1 ring-zinc-200/70 sm:grid-cols-3">
          {page.sections.map((s) => (
            <div key={s.title} className="bg-white p-6">
              <h2 className="text-[15px] font-semibold text-zinc-900">{s.title}</h2>
              <p className="mt-2 text-[14px] leading-relaxed text-zinc-500">{s.body}</p>
            </div>
          ))}
        </div>
        {others.length > 0 ? (
          <div className="mt-10 flex flex-wrap gap-2">
            {others.map((s) => (
              <Link
                key={s.slug}
                href={`${basePath}/${s.slug}`}
                className="rounded-full px-3.5 py-1.5 text-[13px] font-medium text-zinc-600 ring-1 ring-zinc-200 transition-colors hover:bg-zinc-50 hover:text-zinc-950"
              >
                {s.navLabel}
              </Link>
            ))}
          </div>
        ) : null}
      </section>
    </div>
  );
}

// A comparison page: heading, a row-by-row table, and links to the others.
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
      <PageHero title={cmp.title} summary={cmp.summary} ctaHref={ctaHref} />
      <section className={`${WRAP} pb-20`}>
        <div className="overflow-x-auto rounded-2xl ring-1 ring-zinc-200">
          <table className="w-full min-w-[520px] text-[14px]">
            <thead>
              <tr className="border-b border-zinc-200 bg-zinc-50 text-left">
                <th className="px-5 py-3 font-medium text-zinc-500" />
                <th className="px-5 py-3 font-semibold text-zinc-900">{siteConfig.brand.name}</th>
                <th className="px-5 py-3 font-medium text-zinc-500">{cmp.competitor}</th>
              </tr>
            </thead>
            <tbody>
              {cmp.rows.map((r) => (
                <tr key={r.dim} className="border-b border-zinc-100 last:border-0">
                  <td className="px-5 py-3.5 text-zinc-600">{r.dim}</td>
                  <td className="px-5 py-3.5 font-medium text-zinc-900">{r.acme}</td>
                  <td className="px-5 py-3.5 text-zinc-500">{r.them}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {others.length > 0 ? (
          <div className="mt-10 flex flex-wrap gap-2">
            {others.map((c) => (
              <Link
                key={c.slug}
                href={`/compare/${c.slug}`}
                className="rounded-full px-3.5 py-1.5 text-[13px] font-medium text-zinc-600 ring-1 ring-zinc-200 transition-colors hover:bg-zinc-50 hover:text-zinc-950"
              >
                {c.navLabel}
              </Link>
            ))}
          </div>
        ) : null}
      </section>
    </div>
  );
}
