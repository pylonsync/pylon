import React from "react";
import { Link } from "@pylonsync/react";
import type { ProjectView } from "@/lib/agency";

// Reusable presentational pieces for the marketing pages. All server-rendered,
// no client JS. The site is white with black type; `--brand` (set on <html>
// from lib/site.config.ts) colors links only, and `--paper` is the grey band
// behind the contact form. Headlines use `.font-display` (Instrument Serif,
// declared in app.ts).

// Shared container: a contained, centered column.
export const WRAP = "mx-auto w-full max-w-6xl px-6";

// A narrow measure for running text (case studies, intros).
export const WRAP_NARROW = "mx-auto w-full max-w-2xl px-6";

// A text link: black, underlined, brand color on hover.
export const TEXT_LINK =
  "underline decoration-zinc-300 underline-offset-[5px] transition-colors hover:text-brand hover:decoration-brand";

export function Divider() {
  return (
    <div className={WRAP}>
      <div className="border-t border-zinc-200" />
    </div>
  );
}

// A section heading: one large serif line, optionally a short paragraph.
export function SectionTitle({ title, body }: { title: string; body?: string }) {
  return (
    <div>
      <h2 className="font-display text-balance text-[2.25rem] leading-[1.05] tracking-[-0.01em] text-ink sm:text-[2.75rem]">
        {title}
      </h2>
      {body ? <p className="mt-4 max-w-xl text-[16px] leading-relaxed text-zinc-600">{body}</p> : null}
    </div>
  );
}

// A linked portfolio entry: the 4:3 cover, then the title, client label, and
// a one-line summary in small type. Used on the homepage work grid and the
// full /work index, so both stay in lockstep.
export function ProjectCard({ p }: { p: ProjectView }) {
  return (
    <Link href={`/work/${p.slug}`} className="group block">
      {/* Case-study cover: public/images/work/<slug>.jpg. */}
      <ImagePlaceholder shape="landscape" title={`${p.title} product shot`} src={`/images/work/${p.slug}.jpg`} />
      <div className="mt-4 flex items-baseline justify-between gap-4">
        <h3 className="font-display text-[1.5rem] leading-none text-ink transition-colors group-hover:text-brand">
          {p.title}
        </h3>
        <span className="shrink-0 text-[13px] text-zinc-500">
          {p.client}
          {p.year ? `, ${p.year}` : ""}
        </span>
      </div>
      <p className="mt-2 text-[14px] leading-relaxed text-zinc-600">{p.summary}</p>
    </Link>
  );
}

// A clearly marked image placeholder for spots where a real photo belongs:
// dashed border, a photo glyph, and a one-line "swap this" instruction.
//
//   shape  — "landscape" | "portrait" | "square" | "circle"
//   title  — what photo belongs here ("Your headshot")
//   hint   — how to replace it ("Replace in app/page.tsx")
export function ImagePlaceholder({
  shape = "landscape",
  title,
  hint,
  src,
  className = "",
}: {
  shape?: "landscape" | "portrait" | "square" | "circle";
  title: string;
  hint?: string;
  /** A real image. The template ships one under public/images; swap it for yours. */
  src?: string;
  className?: string;
}) {
  const aspect =
    shape === "portrait"
      ? "aspect-[3/4]"
      : shape === "square" || shape === "circle"
        ? "aspect-square"
        : "aspect-[4/3]";
  const radius = shape === "circle" ? "rounded-full" : "rounded-sm";
  if (src) {
    return (
      <div className={`relative overflow-hidden bg-zinc-100 ${aspect} ${radius} ${className}`}>
        <img src={src} alt={title} className="absolute inset-0 size-full object-cover" loading="lazy" />
      </div>
    );
  }
  return (
    <div
      className={`relative grid place-items-center overflow-hidden border border-dashed border-zinc-300 bg-zinc-50 ${aspect} ${radius} ${className}`}
    >
      <div className="px-4 text-center">
        <svg
          className="mx-auto size-7 text-zinc-300"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <circle cx="9" cy="9" r="1.6" />
          <path d="m21 15-4.5-4.5L7 20" />
        </svg>
        <p className="mt-2 text-[12.5px] font-medium text-zinc-500">{title}</p>
        {hint ? <p className="mt-1 text-[11px] leading-snug text-zinc-400">{hint}</p> : null}
      </div>
    </div>
  );
}
